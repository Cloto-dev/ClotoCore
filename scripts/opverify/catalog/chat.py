"""Chat domain — the headline operation-to-success check: a **real LLM chat**
driven all the way to a produced reply.

This runs only in *seed mode* (``bootstrap.prepare_chat_db``), where exactly
one pure-HTTP reasoning engine (default ``deepseek``) is live inside an
isolated copy of a real DB that carries the provider key. Success is not "the
request was accepted" — ``POST /api/chat`` always returns ``{}`` — but that the
agent actually reasoned and emitted a ``ThoughtResponse`` **correlated to our
exact message id**, with non-empty content. That proves the full round trip:
engine subprocess → in-process LLM proxy (key injected) → upstream provider →
back onto the event history.

Correlation is by ``source_message_id`` (a fresh 128-bit id per run), so a
copied history full of old messages can never produce a false pass. The
separate isolation oracle proves no real user DB was touched.
"""

from __future__ import annotations

import secrets
import time
from datetime import datetime, timezone

from . import Operation, RunContext, register

_AGENT = "agent.cloto_default"


def _wait_engine_connected(
    ctx: RunContext, engine_id: str, timeout: float = 90.0
) -> None:
    """Block until the reasoning engine reports ``Connected`` (engines connect
    in a background task after HTTP readiness, so this can lag boot)."""
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        data = ctx.client.get("/api/mcp/servers")
        servers = data.get("servers", []) if isinstance(data, dict) else []
        for s in servers:
            if s.get("id") == engine_id:
                last = s.get("status")
                break
        if last == "Connected":
            return
        time.sleep(1.0)
    raise AssertionError(
        f"engine {engine_id!r} not Connected within {timeout}s (last={last!r})"
    )


class _ChatOp(Operation):
    """Shared driver; subclasses pick the engine (= provider id)."""

    domain = "chat"
    engine_id = "deepseek"
    covers = ["POST /api/chat"]
    phase0 = False  # needs a live LLM engine + key (seed mode only)

    reply_timeout = 120.0

    def drive(self, ctx: RunContext):
        _wait_engine_connected(ctx, self.engine_id)
        nonce = "opv-" + secrets.token_hex(4)
        msg_id = secrets.token_hex(16)
        ts = datetime.now(timezone.utc).isoformat()

        ctx.client.post(
            "/api/chat",
            body={
                "id": msg_id,
                "source": {"type": "User", "id": "opverify", "name": "opverify"},
                "target_agent": _AGENT,
                "content": f"Reply with exactly this token and nothing else: {nonce}",
                "timestamp": ts,
                "metadata": {},
            },
        )

        resp = None
        deadline = time.monotonic() + self.reply_timeout
        while time.monotonic() < deadline:
            history = ctx.client.get("/api/history")
            if isinstance(history, list):
                for ev in history:
                    if ev.get("type") == "ThoughtResponse":
                        d = ev.get("data") or {}
                        if d.get("source_message_id") == msg_id:
                            resp = d
                            break
            if resp:
                break
            time.sleep(2.0)

        return {
            "nonce": nonce,
            "msg_id": msg_id,
            "engine_id": self.engine_id,
            "response": resp,
        }

    def assert_success(self, ctx: RunContext, result):
        resp = result["response"]
        assert resp is not None, (
            f"no ThoughtResponse correlated to message {result['msg_id']} "
            f"(engine {result['engine_id']}) within {self.reply_timeout}s"
        )
        content = (resp.get("content") or "").strip()
        assert content, f"ThoughtResponse content empty: {resp!r}"
        assert resp.get("engine_id") == result["engine_id"], (
            f"reply came from unexpected engine {resp.get('engine_id')!r} "
            f"(wanted {result['engine_id']!r})"
        )
        # Token echo is a soft signal only — models vary in literal compliance;
        # a correlated, non-empty, right-engine reply is the real success.
        if result["nonce"] not in content:
            ctx.log(
                f"chat.{result['engine_id']}: token not echoed verbatim "
                f"(reply={content[:80]!r}); accepted on non-empty correlated reply"
            )


@register
class ChatDeepSeek(_ChatOp):
    name = "deepseek"
    engine_id = "deepseek"
