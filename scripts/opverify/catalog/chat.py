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
_USER = "opverify"


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
    needs_seed = True

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


@register
class ChatMessages(Operation):
    """Chat *persistence* — the transcript store, with no model in the loop.

    ``POST /api/chat/{agent}/messages`` only writes a row (it is the dashboard's
    "remember what was said" path); generation is a different route
    (``POST /api/chat``). So the whole get/post/delete triangle is drivable
    with no LLM, no key and no network, and belongs in the spine slice.

    Success is proved from the read side at every step: the posted message
    comes back out of ``GET`` with the id we minted and the content we sent,
    ``DELETE`` reports how many rows it removed and the subsequent ``GET`` is
    empty. Deletion is scoped to an (agent, user) pair, so the operation also
    writes a message for a *second* user id and asserts it survives the first
    user's delete — a delete that ignored its scope would wipe it too.
    """

    domain = "chat"
    name = "messages"
    covers = [
        "GET /api/chat/{agent_id}/messages",
        "POST /api/chat/{agent_id}/messages",
        "DELETE /api/chat/{agent_id}/messages",
    ]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        mine = "opv-msg-" + secrets.token_hex(8)
        other_user = "opverify-other"
        ctx.scratch["chat_users"] = [_USER, other_user]

        before = _messages(c.get(f"/api/chat/{_AGENT}/messages", params={"user_id": _USER}))

        posted = c.post(
            f"/api/chat/{_AGENT}/messages",
            body={
                "id": mine,
                "source": "user",
                "content": [{"type": "text", "text": "opverify persistence probe"}],
                "user_id": _USER,
            },
        )
        c.post(
            f"/api/chat/{_AGENT}/messages",
            body={
                "id": "opv-other-" + secrets.token_hex(8),
                "source": "user",
                "content": [{"type": "text", "text": "opverify other-user probe"}],
                "user_id": other_user,
            },
        )

        after_post = _messages(
            c.get(f"/api/chat/{_AGENT}/messages", params={"user_id": _USER})
        )
        deleted = c.delete(
            f"/api/chat/{_AGENT}/messages", params={"user_id": _USER}
        )
        after_delete = _messages(
            c.get(f"/api/chat/{_AGENT}/messages", params={"user_id": _USER})
        )
        other_after_delete = _messages(
            c.get(f"/api/chat/{_AGENT}/messages", params={"user_id": other_user})
        )

        # Leave the instance as we found it.
        c.delete(f"/api/chat/{_AGENT}/messages", params={"user_id": other_user})

        return {
            "id": mine,
            "before": before,
            "posted": posted,
            "after_post": after_post,
            "deleted": deleted,
            "after_delete": after_delete,
            "other_after_delete": other_after_delete,
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["before"] == [], (
            f"transcript was not empty before the probe: {result['before']!r}"
        )
        assert (result["posted"] or {}).get("id") == result["id"], (
            f"post did not echo the message id: {result['posted']!r}"
        )
        ids = [m.get("id") for m in result["after_post"]]
        assert ids == [result["id"]], (
            f"stored transcript is {ids!r}, wanted exactly [{result['id']!r}]"
        )
        stored = result["after_post"][0]
        assert "opverify persistence probe" in (stored.get("content") or ""), (
            f"stored content does not carry what was posted: {stored!r}"
        )
        assert stored.get("source") == "user", (
            f"stored message lost its source: {stored!r}"
        )
        assert (result["deleted"] or {}).get("deleted_count") == 1, (
            f"delete reported {result['deleted']!r}, wanted deleted_count=1"
        )
        assert result["after_delete"] == [], (
            f"messages survived the delete: {result['after_delete']!r}"
        )
        assert len(result["other_after_delete"]) == 1, (
            "deleting one user's transcript also removed another user's "
            f"({result['other_after_delete']!r}) — the delete is not scoped "
            f"to (agent, user)"
        )

    def teardown(self, ctx: RunContext):
        for user in ctx.scratch.pop("chat_users", []) or []:
            try:
                ctx.client.delete(
                    f"/api/chat/{_AGENT}/messages", params={"user_id": user}
                )
            except Exception:  # noqa: BLE001
                pass


@register
class ChatRetry(Operation):
    """Re-send a stored user message for regeneration.

    ``retry`` looks the original message up, checks it belongs to the path
    agent (bug-474) and to an enabled agent, then republishes it as a
    ``MessageReceived`` event carrying ``parent_id`` and
    ``skip_user_persist``. The kernel answers as soon as the event is accepted,
    so success is asserted at the route's own boundary: a ``retry_id`` comes
    back and the republished message appears in ``GET /api/history`` correlated
    to the original by ``parent_id`` — not merely that a 200 was returned.

    The two refusals are asserted too, because they are the whole reason the
    handler does a lookup at all: an unknown message id and a *mismatched*
    agent/message pair must both be refused.

    Not phase 0: accepting the event hands it to the agentic loop, which then
    tries to reason. On a fresh instance no engine is registered, and the
    resulting failure is logged at a level the run's log oracle treats as a
    fault. It belongs with the other operations that assume a live engine.
    """

    domain = "chat"
    name = "retry"
    covers = ["POST /api/chat/{agent_id}/messages/{message_id}/retry"]
    phase0 = False
    # The accepted event lands in the agentic loop; without a live engine the
    # loop's failure is logged at a level the run's log oracle reads as a fault.
    needs_seed = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        msg_id = "opv-retry-" + secrets.token_hex(8)
        ctx.scratch["retry_user"] = _USER
        c.post(
            f"/api/chat/{_AGENT}/messages",
            body={
                "id": msg_id,
                "source": "user",
                "content": [{"type": "text", "text": "opverify retry probe"}],
                "user_id": _USER,
            },
        )

        accepted = c.post(f"/api/chat/{_AGENT}/messages/{msg_id}/retry", timeout=30.0)

        republished = None
        deadline = time.monotonic() + 15.0
        while time.monotonic() < deadline:
            history = ctx.client.get("/api/history")
            if isinstance(history, list):
                for ev in history:
                    if ev.get("type") != "MessageReceived":
                        continue
                    meta = (ev.get("data") or {}).get("metadata") or {}
                    if meta.get("parent_id") == msg_id:
                        republished = ev.get("data")
                        break
            if republished:
                break
            time.sleep(0.5)

        unknown_status, _ = c.request_raw(
            "POST", f"/api/chat/{_AGENT}/messages/opv-does-not-exist/retry"
        )
        mismatch_status, _ = c.request_raw(
            "POST", f"/api/chat/agent.opverify_absent/messages/{msg_id}/retry"
        )

        c.delete(f"/api/chat/{_AGENT}/messages", params={"user_id": _USER})
        return {
            "msg_id": msg_id,
            "accepted": accepted,
            "republished": republished,
            "unknown_status": unknown_status,
            "mismatch_status": mismatch_status,
        }

    def assert_success(self, ctx: RunContext, result):
        assert (result["accepted"] or {}).get("retry_id"), (
            f"retry returned no retry_id: {result['accepted']!r}"
        )
        resp = result["republished"]
        assert resp is not None, (
            f"no MessageReceived event carrying parent_id={result['msg_id']} "
            f"reached the history — the retry was accepted but never dispatched"
        )
        assert (resp.get("content") or "").strip() == "opverify retry probe", (
            f"the republished message lost the original text: {resp!r}"
        )
        meta = resp.get("metadata") or {}
        assert meta.get("skip_user_persist") == "true", (
            f"retry did not mark the message as already-persisted: {meta!r} "
            f"(without it the retry duplicates the user's turn)"
        )
        assert result["unknown_status"] == 404, (
            f"retry of an unknown message id answered "
            f"{result['unknown_status']}, wanted 404"
        )
        assert result["mismatch_status"] == 404, (
            f"retry with a mismatched agent answered "
            f"{result['mismatch_status']}, wanted 404 (bug-474: a message must "
            f"not be re-injectable into another agent's stream)"
        )

    def teardown(self, ctx: RunContext):
        user = ctx.scratch.pop("retry_user", None)
        if user:
            try:
                ctx.client.delete(
                    f"/api/chat/{_AGENT}/messages", params={"user_id": user}
                )
            except Exception:  # noqa: BLE001
                pass


def _messages(payload):
    """The ``messages`` array out of a get_messages response."""
    if isinstance(payload, dict):
        return payload.get("messages") or []
    return payload or []
