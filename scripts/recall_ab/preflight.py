#!/usr/bin/env python3
"""Pre-flight check for the recall-contamination A/B harness.

Sends ONE probe to the target agent and fails loudly if the reply looks like an
engine-resolution error. This guards against bug-396: a test agent whose
``default_engine_id`` is an id the kernel cannot resolve (e.g. ``mind.deepseek``
after the catalog renamed the connector to ``deepseek``) makes every turn return
the literal string ``[Error] Engine '...' not found``. The A/B classifier scores
that as ``coherent`` — a *false 0% drift* that silently corrupts the whole run
(this is exactly what wrecked the first Phase-5 pass on 2026-06-14).

Run this before spending ~84 LLM calls on a full OLD/NEW A/B:

    uv run --python 3.13 --with httpx python preflight.py \
      --agent-id agent.abtest --api-key "$CLOTO_API_KEY"

Exit codes: 0 = engine resolves, 2 = no reply (kernel/engine/embedding down),
3 = engine did not resolve (fix the test agent's default_engine_id).

It reuses run_ab.py's config / SSE bus / send path, so it accepts the same
flags (``--base-url`` / ``--api-key`` / ``--agent-id`` / ``--source-id`` /
``--channel`` / ``--response-timeout``).
"""

from __future__ import annotations

import sys
import uuid

import httpx

from run_ab import ResponseBus, load_config, send_and_wait

# A reply is treated as an engine-resolution failure when it carries an error
# marker AND names the engine machinery — conservative, so a normal answer that
# merely contains "not found" in prose is not flagged.
_ERROR_MARKERS = ("[error]", "not found", "no engine", "unresolved")


def looks_like_engine_error(resp: str) -> bool:
    low = resp.lower()
    has_marker = any(m in low for m in _ERROR_MARKERS)
    names_engine = "engine" in low
    return has_marker and names_engine


def main() -> int:
    cfg = load_config()
    bus = ResponseBus(cfg.base_url, cfg.api_key)
    bus.start()
    try:
        with httpx.Client() as client:
            session_id = f"abtest-preflight-{uuid.uuid4().hex[:8]}"
            resp = send_and_wait(
                client,
                bus,
                cfg,
                "preflight check: reply with one short sentence.",
                session_id,
                timeout=cfg.response_timeout,
            )
    finally:
        bus.stop()

    if resp is None:
        print(
            "PREFLIGHT FAIL (2): no reply within "
            f"{cfg.response_timeout}s. The kernel, the agent's reasoning engine, "
            "or the embedding server is not responding. Check the kernel is up at "
            f"{cfg.base_url} and the test agent '{cfg.agent_id}' has an engine + "
            "memory.cpersona granted and the embedding server running.",
            file=sys.stderr,
        )
        return 2

    if looks_like_engine_error(resp):
        print(
            "PREFLIGHT FAIL (3): the agent's engine did not resolve (bug-396). "
            f"Reply was:\n  {resp!r}\n"
            f"Fix: set the test agent '{cfg.agent_id}' default_engine_id to the "
            "engine's RESOLVABLE MCP server-name (the kernel matches engine ids "
            "by exact server name) -- e.g. 'deepseek', NOT 'mind.deepseek'. "
            "Running the A/B in this state produces a false 0% drift.",
            file=sys.stderr,
        )
        return 3

    print(
        f"PREFLIGHT OK: agent '{cfg.agent_id}' engine resolves and replied.\n"
        f"  {resp[:200]}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
