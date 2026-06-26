#!/usr/bin/env python3
"""Recall-contamination A/B harness for the Discord-recall redesign (Phase 5).

Measures topic-drift ("pan -> raspberry pie") severity on a running ClotoCore
kernel, so the OLD behaviour (recall every turn) can be compared against the
NEW behaviour (session-start-gated recall + per-channel episodic loop). It is
the redesign-era successor to the one-off harness described in
docs/RECALL_CONTAMINATION_AB_2026-04-24.md.

What it does
------------
1. (optional, --seed) Plants the seed corpus as per-user memories by sending
   each entry as a USER message in its own throwaway session, so the kernel
   stores it via the normal per-turn store path.
2. Runs the 14-query probe set as an ACCUMULATING conversation in a single
   session, repeated N_TRIALS times (one fresh session per trial). The single
   accumulating session is what lets the NEW arm's gate matter: only the first
   turn is a session-start, so later turns do not auto-recall.
3. Captures each response via the SSE event stream (ThoughtResponse correlated
   by source_message_id) and classifies it coherent / mild / severe drift.
4. Writes results_<arm>.json and prints a summary table.

Arms are selected by which kernel build is running (this script does not switch
behaviour) — pass --arm old|new (or CLOTO_AB_ARM) purely to label the output.

It performs NO destructive DB operations. For clean arms, snapshot/restore the
target agent's CPersona rows yourself between runs (see README.md).

Transport facts (verified against the kernel source, 2026-06-14):
- POST {base}/api/chat   body = a full ClotoMessage JSON; fire-and-forget.
- GET  {base}/api/events?token=<key>   SSE; the reply arrives as an event
  {"type":"ThoughtResponse","data":{"content","source_message_id",...}}.
- Auth: X-API-Key header (POST) / ?token= query (SSE). Omit if the kernel has
  no admin_api_key configured.

Run:  uv run --with httpx python run_ab.py --seed         # once, to plant memories
      uv run --with httpx python run_ab.py --arm old      # against the OLD build
      uv run --with httpx python run_ab.py --arm new      # against the NEW build
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import sys
import threading
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path

import httpx

HERE = Path(__file__).resolve().parent


# --------------------------------------------------------------------------- #
# Config
# --------------------------------------------------------------------------- #
def load_config() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Recall-contamination A/B harness")
    p.add_argument("--base-url", default=os.environ.get("CLOTO_BASE_URL", "http://127.0.0.1:8081"))
    p.add_argument("--api-key", default=os.environ.get("CLOTO_API_KEY", ""))
    p.add_argument("--agent-id", default=os.environ.get("CLOTO_AB_AGENT", "agent.cloto_default"))
    p.add_argument("--arm", default=os.environ.get("CLOTO_AB_ARM", "unset"),
                   help="label for the output file (old|new|...); does not change behaviour")
    p.add_argument("--source-id", default=os.environ.get("CLOTO_AB_SOURCE_ID", "abtest:user1"),
                   help="source.id for User messages (must match between seed and probe)")
    p.add_argument("--channel", default=os.environ.get("CLOTO_AB_CHANNEL", "chat"),
                   help="external_source value (memory channel): chat | discord")
    p.add_argument("--trials", type=int, default=int(os.environ.get("CLOTO_AB_TRIALS", "3")))
    p.add_argument("--response-timeout", type=float,
                   default=float(os.environ.get("CLOTO_AB_RESPONSE_TIMEOUT", "90")))
    p.add_argument("--seed", action="store_true", help="plant the seed corpus, then exit")
    p.add_argument("--out-dir", default=os.environ.get("CLOTO_AB_OUT", str(HERE / "results")))
    return p.parse_args()


# --------------------------------------------------------------------------- #
# SSE listener — captures ThoughtResponse events keyed by source_message_id
# --------------------------------------------------------------------------- #
class ResponseBus:
    def __init__(self, base_url: str, api_key: str):
        self._base = base_url.rstrip("/")
        self._key = api_key
        self._responses: dict[str, str] = {}
        self._lock = threading.Lock()
        self._ready = threading.Event()
        self._stop = threading.Event()
        self._err: queue.Queue[str] = queue.Queue()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self) -> None:
        self._thread.start()
        if not self._ready.wait(timeout=15):
            extra = ""
            try:
                extra = f" ({self._err.get_nowait()})"
            except queue.Empty:
                pass
            raise RuntimeError(f"SSE stream did not connect within 15s{extra}")

    def stop(self) -> None:
        self._stop.set()

    def _run(self) -> None:
        url = f"{self._base}/api/events"
        params = {"token": self._key} if self._key else {}
        try:
            # No read timeout: the stream is long-lived.
            with httpx.Client(timeout=httpx.Timeout(10.0, read=None)) as client:
                with client.stream("GET", url, params=params) as resp:
                    if resp.status_code != 200:
                        self._err.put(f"HTTP {resp.status_code} from {url}")
                        return
                    data_lines: list[str] = []
                    for line in resp.iter_lines():
                        if self._stop.is_set():
                            return
                        if line == "":  # event boundary
                            self._dispatch(data_lines)
                            data_lines = []
                            continue
                        if line.startswith("event:") and "handshake" in line:
                            self._ready.set()
                        elif line.startswith("data:"):
                            data_lines.append(line[5:].lstrip())
        except Exception as e:  # noqa: BLE001 — surface any transport failure to the main thread
            self._err.put(repr(e))
            self._ready.set()  # unblock start() so it can raise

    def _dispatch(self, data_lines: list[str]) -> None:
        if not data_lines:
            return
        raw = "\n".join(data_lines)
        if raw == "connected":
            self._ready.set()
            return
        try:
            obj = json.loads(raw)
        except json.JSONDecodeError:
            return
        if obj.get("type") != "ThoughtResponse":
            return
        data = obj.get("data") or {}
        smid = data.get("source_message_id")
        content = data.get("content", "")
        if smid:
            with self._lock:
                self._responses[smid] = content

    def wait_for(self, msg_id: str, timeout: float) -> str | None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self._lock:
                if msg_id in self._responses:
                    return self._responses.pop(msg_id)
            time.sleep(0.2)
        return None


# --------------------------------------------------------------------------- #
# Sending
# --------------------------------------------------------------------------- #
def _now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def send_and_wait(
    client: httpx.Client,
    bus: ResponseBus,
    cfg: argparse.Namespace,
    content: str,
    session_id: str,
    timeout: float,
) -> str | None:
    msg_id = f"abtest-{uuid.uuid4().hex}"
    body = {
        "id": msg_id,
        "source": {"type": "User", "id": cfg.source_id, "name": "ABTester"},
        "target_agent": cfg.agent_id,
        "content": content,
        "timestamp": _now_iso(),
        "metadata": {
            "external_session_id": session_id,
            "external_source": cfg.channel,
            "target_agent_id": cfg.agent_id,
        },
    }
    headers = {"X-API-Key": cfg.api_key} if cfg.api_key else {}
    r = client.post(f"{cfg.base_url.rstrip('/')}/api/chat", json=body, headers=headers, timeout=30.0)
    r.raise_for_status()
    return bus.wait_for(msg_id, timeout)


# --------------------------------------------------------------------------- #
# Classification — heuristic approximation of the §2.3 rubric.
#   severe  = unrelated topic present AND elaborated (>=3 mentions or a question)
#   mild    = unrelated topic present but disclaimed
#   coherent= no unrelated topic (or disclaimed without elaboration)
# Manual review remains the gold standard; see README "Classifier caveat".
# --------------------------------------------------------------------------- #
_DISCLAIMERS = ["以前", "別の話", "前に話した", "とは別", "今回は関係", "余談", "別件"]


def classify(resp: str | None, q: dict, memory_topics: list[str]) -> str:
    if resp is None:
        return "timeout"
    contaminants = memory_topics if q.get("false_positive") else q.get("contaminant_keywords", [])
    hits = [kw for kw in contaminants if kw and kw in resp]
    if not hits:
        return "coherent"
    # A disclaiming reference ("以前話した…とは別") is the rubric's MILD signal.
    if any(d in resp for d in _DISCLAIMERS):
        return "mild"
    # Count the most-mentioned single keyword (not the sum — overlapping
    # substrings like ラズベリーパイ ⊃ ラズベリー ⊃ パイ would triple-count one mention).
    mentions = max((resp.count(kw) for kw in hits), default=0)
    elaborated = mentions >= 3 or "?" in resp or "？" in resp
    return "severe" if elaborated else "mild"


# --------------------------------------------------------------------------- #
# Phases
# --------------------------------------------------------------------------- #
def do_seed(client: httpx.Client, bus: ResponseBus, cfg: argparse.Namespace) -> None:
    corpus = json.loads((HERE / "seed_corpus.json").read_text(encoding="utf-8"))["memories"]
    print(f"Seeding {len(corpus)} memories into '{cfg.agent_id}' (source_id={cfg.source_id}) ...")
    for i, mem in enumerate(corpus):
        sess = f"abtest-seed-{i}-{uuid.uuid4().hex[:8]}"
        resp = send_and_wait(client, bus, cfg, mem, sess, timeout=cfg.response_timeout)
        status = "ok" if resp is not None else "no-reply(stored anyway)"
        print(f"  [{i + 1}/{len(corpus)}] {status}: {mem[:32]}...")
    # The store call is spawned during handling; give it a moment to flush.
    time.sleep(3.0)
    print("Seeding complete. Snapshot the agent's CPersona rows now, then run each arm.")


def do_probe(client: httpx.Client, bus: ResponseBus, cfg: argparse.Namespace) -> dict:
    spec = json.loads((HERE / "query_set.json").read_text(encoding="utf-8"))
    queries = spec["queries"]
    memory_topics = spec["memory_topics"]
    cells: dict[str, list[str]] = {q["id"]: [] for q in queries}

    for trial in range(cfg.trials):
        session_id = f"abtest-probe-{cfg.arm}-t{trial}-{uuid.uuid4().hex[:8]}"
        print(f"\n--- trial {trial + 1}/{cfg.trials}  session={session_id} ---")
        for q in queries:
            resp = send_and_wait(client, bus, cfg, q["text"], session_id, timeout=cfg.response_timeout)
            verdict = classify(resp, q, memory_topics)
            cells[q["id"]].append(verdict)
            print(f"  {q['id']:>3} [{verdict:<8}] {q['text']}")

    return summarize(cfg, queries, cells)


def summarize(cfg: argparse.Namespace, queries: list[dict], cells: dict[str, list[str]]) -> dict:
    counts = {"coherent": 0, "mild": 0, "severe": 0, "timeout": 0, "error": 0}
    for verdicts in cells.values():
        for v in verdicts:
            counts[v] = counts.get(v, 0) + 1
    completed = counts["coherent"] + counts["mild"] + counts["severe"]
    severe_pct = (100.0 * counts["severe"] / completed) if completed else 0.0
    return {
        "arm": cfg.arm,
        "agent_id": cfg.agent_id,
        "channel": cfg.channel,
        "trials": cfg.trials,
        "generated_at": _now_iso(),
        "counts": counts,
        "severe_pct_of_completed": round(severe_pct, 1),
        "per_query": {q["id"]: {"text": q["text"], "category": q["category"],
                                "verdicts": cells[q["id"]]} for q in queries},
    }


def print_summary(result: dict) -> None:
    c = result["counts"]
    print("\n========== SUMMARY ==========")
    print(f"arm={result['arm']}  agent={result['agent_id']}  trials={result['trials']}")
    print(f"coherent={c['coherent']} mild={c['mild']} severe={c['severe']} "
          f"timeout={c['timeout']} error={c['error']}")
    print(f"SEVERE drift rate (of completed): {result['severe_pct_of_completed']}%")
    print("per-query (C=coherent m=mild S=severe T=timeout E=error):")
    short = {"coherent": "C", "mild": "m", "severe": "S", "timeout": "T", "error": "E"}
    for qid, info in result["per_query"].items():
        marks = "".join(short.get(v, "?") for v in info["verdicts"])
        print(f"  {qid:>3} {marks:<6} {info['text']}")


# --------------------------------------------------------------------------- #
# Main
# --------------------------------------------------------------------------- #
def main() -> int:
    cfg = load_config()
    bus = ResponseBus(cfg.base_url, cfg.api_key)
    try:
        bus.start()
    except RuntimeError as e:
        print(f"ERROR: {e}", file=sys.stderr)
        print("Is the kernel running and is the API key correct? "
              "(set --base-url / --api-key or CLOTO_BASE_URL / CLOTO_API_KEY)", file=sys.stderr)
        return 1

    with httpx.Client() as client:
        if cfg.seed:
            do_seed(client, bus, cfg)
            bus.stop()
            return 0
        result = do_probe(client, bus, cfg)

    bus.stop()
    print_summary(result)
    out_dir = Path(cfg.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"results_{cfg.arm}.json"
    out_path.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\nWrote {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
