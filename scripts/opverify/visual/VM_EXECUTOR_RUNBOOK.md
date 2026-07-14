# Visual apex — two-tier orchestration (orchestrator ↔ VM executor)

The visual apex is driven by **two cooperating models**, not one. This split is a
permanent part of the opverify pipeline (adopted 2026-07-14): it keeps the
orchestrator's context lean while a cheaper multimodal model does the heavy,
repetitive VM I/O — and it *is* the "live multimodal assessor" the bootstrap
`RecordedVision` was always meant to become (see `run_vm.py`).

```
┌─ Orchestrator (Opus, main loop) ──────────────────────────────────────┐
│  • designs the journey + the dual-oracle assertions                    │
│  • writes the executor's task from THIS runbook                        │
│  • VERIFIES the executor's verdict (re-runs the deterministic kernel   │
│    correlation itself + eyeballs the critical frame(s))                │
│  • records the result (CSC / ledger) and decides next steps            │
└───────────────────────────────────────────────────────────────────────┘
                    │ spawns (Agent, model=sonnet, ~mid effort)
                    ▼
┌─ VM executor (Sonnet, subagent) ──────────────────────────────────────┐
│  • actuates the GUI (click/type/key via the session-1 agent)          │
│  • perceives (grabs frames, READS the pixels — first-pass visual       │
│    assessment: rendered? defect? spinner stuck?)                       │
│  • probes the kernel (health / chat / history)                        │
│  • returns a COMPACT structured verdict + key frame paths (never the   │
│    raw PNG bytes or full history) so the orchestrator stays lean       │
└───────────────────────────────────────────────────────────────────────┘
```

Why two tiers: every `/grab` is a ~40 KB PNG the visual oracle must *read*, and a
journey is many grab→read→act cycles. Doing that in the orchestrator's own
context bloats it fast. Delegating the cycle to a Sonnet subagent isolates that
cost; the orchestrator pays only for the design + the compact verdict + a
spot-check. Cost pools also differ, and the executor's visual read is a genuine
intelligent perceiver — exactly the apex's design (a VLM in the human perception
loop), just hosted in a subagent.

## Two ways the executor drives

- **Manual VM I/O** (the flow detailed below): the subagent curl/tunnel-grabs,
  Reads the PNG, judges, curl-acts, curl-probes — assembling the dual-oracle
  verdict itself. Maximum flexibility for exploratory / ad-hoc journeys.
- **Structured handshake** (`OPV_ASSESSOR=handshake`, #237): the subagent runs
  `python -m scripts.opverify.visual.run_vm <journey>` in the background and only
  supplies the *visual* verdict per frame — the driver owns the loop, journey
  structure, dual-oracle cross-check, tiering and forensic capture. Prefer this
  to run a *defined* journey with less hand-rolling. Protocol: the driver writes
  `req_NNN.json` `{seq, question, frame}` into `OPV_EXCHANGE_DIR`; you Read the
  referenced PNG (your live eyes) and Write `resp_NNN.json`
  `{visible: bool, detail: str, defects: [str]}`; repeat until `done.flag`
  appears, then read the run's JSON report. This is the live replacement for the
  `RecordedVision` bootstrap — the executor's read *is* the oracle.

## Orchestrator responsibilities (do NOT delegate these)

1. **Journey + oracle design.** Decide the steps, the visual question at each
   checkpoint, and the kernel hard-gate it cross-checks against.
2. **Verification of the verdict (mandatory).** The executor's report is a
   *claim*. Before accepting AGREE_PASS:
   - Re-run the **deterministic** kernel oracle yourself (e.g. `GET /api/history`
     and confirm the correlated `ThoughtResponse` — engine id + content). This is
     cheap and non-negotiable (delegation-verification discipline).
   - **Eyeball the critical frame(s)** the executor saved (the response render at
     minimum). The visual oracle is the apex's whole point; don't take it on
     faith.
3. **Record + decide.** Log the outcome (CSC Goal #170 / ledger), file any real
   defect, choose the next journey.

## VM executor task template (hand this, filled in, to the subagent)

Spawn with `Agent(subagent_type="general-purpose", model="sonnet")`. The prompt
must contain, verbatim, the operational runbook below plus the journey-specific
goal + dual-oracle spec.

### VM access (stable facts)
- **Canonical automated transport:** the committed harness
  (`python -m scripts.opverify.visual.run_vm <journey>`, backend
  `backends_vm.py`) drives the guest via bare `curl.exe` over an SSH
  connection kept warm with `ControlMaster`/`ControlPersist` — only the first
  call pays the TCP+auth handshake (measured ~0.76s→~0.4s per round trip). Grab
  bytes come back raw (no base64), POST bodies ride ssh stdin. Prefer this for
  anything scripted.
- **Manual round trips:** run `curl.exe` directly over a multiplexed ssh —
  `ssh -o ControlMaster=auto -o ControlPersist=60 -o ControlPath=/tmp/opv-ssh-%r@%h:%p
  PC@192.168.0.252 'curl.exe -s http://127.0.0.1:8900/grab' > frame.png`. Reuse
  the same `-o ControlPath` on every call so the handshake is paid once. The
  legacy `scratchpad/vmps.sh` (`ssh … powershell -EncodedCommand`, UTF-16LE
  base64) remains only for ad-hoc PowerShell that isn't a simple HTTP call.
- **Admin token:** `scratchpad/opv_key.txt` = kernel `CLOTO_API_KEY`. Use for the
  `X-API-Key` header; transport base64 (`$ak =
  [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String("<b64>"))`) so it
  never appears in plaintext. **Never print it.**
- **Actuator agent** (session 1, no auth): `http://127.0.0.1:8900` — `GET /grab`
  (image/png), `POST /act` (`{kind:"click",x,y}` / `{kind:"type",text}` /
  `{kind:"key",key:"enter"}` / `hotkey` / `scroll`), `GET /health`, `POST /run`
  (`{path,args,env}` — env merges over the process env, e.g. to inject a known
  `CLOTO_API_KEY` when launching the app).
- **Kernel admin API** (needs `X-API-Key`): `http://127.0.0.1:8081` —
  `/api/system/health`, `/api/chat`, `/api/history`, `/api/llm/providers`,
  `/api/mcp/servers`, `/api/system/version`.
- **Screen** 1280×800.
- **Grab→read:** `ssh <multiplexed-opts> PC@192.168.0.252 'curl.exe -s
  http://127.0.0.1:8900/grab' > frame.png`, then Read the PNG (raw bytes, no
  base64 step). The legacy PS `[Convert]::ToBase64String($r.Content) | tr -d
  '\r\n' | base64 -d` still works if you're already in a PS snippet.
- To settle before a checkpoint, `POST /act` then sleep ~1.5s then `/grab`;
  with a warm `ControlPath` each of those is a cheap reuse, so separate calls
  are fine (no need to cram them into one PS to save handshakes).

### Chat payload / oracle (reference)
- `POST /api/chat` body: `{id, source:{type:"User",id,name}, target_agent:
  "agent.cloto_default", content, timestamp, metadata:{}}`. Returns `{}`
  (accept ≠ success).
- Success = a `ThoughtResponse` in `/api/history` (`{"data":[...]}`) with
  `data.source_message_id == id`, non-empty `data.content`, expected
  `data.engine_id`. When driving via the **GUI** you don't know the frontend's
  msg id — correlate by a unique **nonce** appearing in the response content.

### Security (hard rules)
- MUST NOT read/print/log any **LLM provider API key** contents. Only the boolean
  `has_key` may surface.
- The admin token is auth-only — use, never print.

### Return schema (compact — the only thing the orchestrator ingests)
```json
{
  "reached_target": true,
  "steps_taken": 0,
  "visual": {"rendered": true, "detail": "...", "defects": []},
  "kernel": {"ok": true, "engine_id": "...", "content_snippet": "..."},
  "diagnosis": "AGREE_PASS|FRONTEND_BUG|BACKEND_OR_HIDDEN|AGREE_FAIL",
  "key_frames": {"label": "<path>"},
  "notes": "anything the orchestrator should verify or that went wrong"
}
```
Do not paste PNG bytes or full history. Stop after a reasonable attempt if
blocked and return what you have with the blocker in `notes`.

## Diagnosis codes (shared vocabulary)
- **AGREE_PASS** — visual rendered ✓ and kernel confirms ✓.
- **FRONTEND_BUG** — kernel ✓ but the GUI didn't render it (soft/warn; the class
  the apex exists to catch).
- **BACKEND_OR_HIDDEN** — GUI shows it but the kernel has no record (hard-fail).
- **AGREE_FAIL** — neither.

## Known harness artifacts (not product bugs)
- **Keystroke fidelity:** the session-1 agent types via pyautogui `write()`, which
  mis-maps some shifted characters on a JP keyboard layout (observed 2026-07-14:
  `:` → `*` in the sent text). It does not affect alphanumeric nonces. For exact
  literal text, prefer clipboard paste over simulated keystrokes.
