"""Conversations domain — the six routes behind the sidebar's threads
(docs/CONVERSATIONS_DESIGN.md §3), driven to observable effects.

Every operation makes a throwaway agent of its own, so the counts it asserts
are its own: the bulk routes act on an agent's conversations for a user *and*
on the 'system' user's, and the default agent may carry either in a seeded run.
No LLM is involved — a conversation exists before anything is said in it — so
both are phase-0.

Success is never "the call returned". A rename is proved by the list showing
the new title, an archive by the conversation leaving the default list and
staying in the archived one, a delete by the next touch answering 404, and a
bulk action by the count it reports matching what the lists show afterwards.
"""

from __future__ import annotations

import json
import secrets

from . import Operation, RunContext, register

_USER = "opverify"


def _make_agent(ctx: RunContext, key: str) -> str:
    created = ctx.client.post(
        "/api/agents",
        body={
            "name": f"opverify-conversations-{secrets.token_hex(3)}",
            "description": "opverify conversations probe agent",
            "default_engine": "cerebras",
        },
    )
    agent_id = created["id"]
    ctx.scratch[key] = agent_id
    return agent_id


def _drop_agent(ctx: RunContext, key: str) -> None:
    agent_id = ctx.scratch.pop(key, None)
    if not agent_id:
        return
    try:
        ctx.client.post(f"/api/chat/{agent_id}/conversations/delete-all", body={"user_id": _USER})
    except Exception:
        pass
    try:
        ctx.client.delete(f"/api/agents/{agent_id}")
    except Exception:
        pass


def _list(ctx: RunContext, agent_id: str, archived: bool = False):
    params = {"user_id": _USER}
    if archived:
        params["include_archived"] = "true"
    return ctx.client.get(f"/api/chat/{agent_id}/conversations", params=params)["conversations"]


def _ids(rows):
    return {r["id"] for r in rows}


@register
class ConversationsLifecycle(Operation):
    domain = "conversations"
    name = "lifecycle"
    covers = [
        "POST /api/chat/{agent_id}/conversations",
        "GET /api/chat/{agent_id}/conversations",
        "PATCH /api/chat/{agent_id}/conversations/{conversation_id}",
        "DELETE /api/chat/{agent_id}/conversations/{conversation_id}",
    ]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        agent_id = _make_agent(ctx, "conversations_agent")
        base = f"/api/chat/{agent_id}/conversations"

        created = c.post(base, body={"user_id": _USER})
        conv_id = created["id"]
        listed = _list(ctx, agent_id)

        renamed = c.request("PATCH", f"{base}/{conv_id}", body={"title": "  opverify thread  "})
        after_rename = _list(ctx, agent_id)

        archived = c.request("PATCH", f"{base}/{conv_id}", body={"archived": True})
        live_after_archive = _list(ctx, agent_id)
        all_after_archive = _list(ctx, agent_id, archived=True)

        restored = c.request("PATCH", f"{base}/{conv_id}", body={"archived": False})
        live_after_restore = _list(ctx, agent_id)

        blank_status, _ = c.request_raw("PATCH", f"{base}/{conv_id}", body={"title": "   "})
        # An id is not enough to reach another agent's conversation.
        foreign_status, _ = c.request_raw(
            "PATCH", f"/api/chat/agent.not-the-owner/conversations/{conv_id}", body={"title": "taken"}
        )
        unknown_agent_status, _ = c.request_raw(
            "POST", "/api/chat/agent.does-not-exist/conversations", body={"user_id": _USER}
        )

        deleted = c.delete(f"{base}/{conv_id}")
        all_after_delete = _list(ctx, agent_id, archived=True)
        touch_after_delete, _ = c.request_raw("PATCH", f"{base}/{conv_id}", body={"title": "again"})

        return {
            "created": created,
            "listed_ids": _ids(listed),
            "renamed": renamed,
            "title_in_list": next((r["title"] for r in after_rename if r["id"] == conv_id), None),
            "archived": archived,
            "live_after_archive": _ids(live_after_archive),
            "archived_row": next((r for r in all_after_archive if r["id"] == conv_id), None),
            "restored": restored,
            "live_after_restore": _ids(live_after_restore),
            "blank_status": blank_status,
            "foreign_status": foreign_status,
            "unknown_agent_status": unknown_agent_status,
            "deleted": deleted,
            "all_after_delete": _ids(all_after_delete),
            "touch_after_delete": touch_after_delete,
            "agent_id": agent_id,
        }

    def assert_success(self, ctx: RunContext, result):
        created = result["created"]
        conv_id = created["id"]
        assert conv_id, f"create returned no id: {created!r}"
        assert created["agent_id"] == result["agent_id"], f"created on the wrong agent: {created!r}"
        assert created["title"] == "" and created["archived_at"] is None, f"not a fresh conversation: {created!r}"
        assert conv_id in result["listed_ids"], "created conversation missing from the list"

        assert result["renamed"]["title"] == "opverify thread", f"rename not trimmed/applied: {result['renamed']!r}"
        assert result["title_in_list"] == "opverify thread", f"list shows title {result['title_in_list']!r}"

        assert result["archived"]["archived_at"] is not None, f"archive set no time: {result['archived']!r}"
        assert conv_id not in result["live_after_archive"], "archived conversation still in the live list"
        row = result["archived_row"]
        assert row is not None and row["archived_at"] is not None, "archived conversation not kept in the archive"

        assert result["restored"]["archived_at"] is None, f"unarchive left a time: {result['restored']!r}"
        assert conv_id in result["live_after_restore"], "unarchived conversation not back in the live list"

        assert result["blank_status"] == 400, f"blank title accepted: HTTP {result['blank_status']}"
        assert result["foreign_status"] == 404, f"another agent reached it: HTTP {result['foreign_status']}"
        assert result["unknown_agent_status"] == 404, (
            f"created for an agent that does not exist: HTTP {result['unknown_agent_status']}"
        )

        assert result["deleted"].get("deleted_messages") == 0, f"delete reported {result['deleted']!r}"
        assert conv_id not in result["all_after_delete"], "deleted conversation still listed (archived included)"
        assert result["touch_after_delete"] == 404, f"deleted conversation still answers: HTTP {result['touch_after_delete']}"

    def teardown(self, ctx: RunContext):
        _drop_agent(ctx, "conversations_agent")


@register
class ConversationsBulk(Operation):
    domain = "conversations"
    name = "bulk"
    covers = [
        "POST /api/chat/{agent_id}/conversations/archive-all",
        "POST /api/chat/{agent_id}/conversations/delete-all",
    ]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        agent_id = _make_agent(ctx, "conversations_bulk_agent")
        base = f"/api/chat/{agent_id}/conversations"

        made = [c.post(base, body={"user_id": _USER})["id"] for _ in range(3)]
        already = c.post(base, body={"user_id": _USER})["id"]
        c.request("PATCH", f"{base}/{already}", body={"archived": True})

        archive_all = c.post(f"{base}/archive-all", body={"user_id": _USER})
        live_after_archive = _list(ctx, agent_id)
        all_after_archive = _list(ctx, agent_id, archived=True)

        live_again = c.post(base, body={"user_id": _USER})["id"]
        delete_all = c.post(f"{base}/delete-all", body={"user_id": _USER})
        all_after_delete = _list(ctx, agent_id, archived=True)

        return {
            "made": made,
            "already": already,
            "archive_all": archive_all,
            "live_after_archive": _ids(live_after_archive),
            "all_after_archive": _ids(all_after_archive),
            "live_again": live_again,
            "delete_all": delete_all,
            "all_after_delete": _ids(all_after_delete),
        }

    def assert_success(self, ctx: RunContext, result):
        # The one archived beforehand is not counted again.
        assert result["archive_all"] == {"archived": 3}, f"archive-all reported {json.dumps(result['archive_all'])}"
        assert result["live_after_archive"] == set(), f"live after archive-all: {result['live_after_archive']!r}"
        expected = set(result["made"]) | {result["already"]}
        assert result["all_after_archive"] == expected, (
            f"archive kept {result['all_after_archive']!r}, expected {expected!r}"
        )
        # Delete-all takes the archived ones too.
        assert result["delete_all"] == {"deleted": 5}, f"delete-all reported {json.dumps(result['delete_all'])}"
        assert result["all_after_delete"] == set(), f"left after delete-all: {result['all_after_delete']!r}"

    def teardown(self, ctx: RunContext):
        _drop_agent(ctx, "conversations_bulk_agent")


@register
class ConversationsPanelRoutes(Operation):
    """The two routes a connector panel is given (docs/PANEL_WRITE_GATE_DESIGN.md
    §4.2): one conversation read by path, and speaking to the agent in the path.

    A successful send reaches the agent's engine, which phase-0 does not have, so
    here the send is driven only to the answers that must stop it before the
    agent loop: nothing to say, and a conversation of another agent or none."""

    domain = "conversations"
    name = "panel-routes"
    covers = [
        "GET /api/chat/{agent_id}/conversations/{conversation_id}",
        "POST /api/chat/{agent_id}/send",
    ]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        agent_id = _make_agent(ctx, "conversations_panel_agent")
        base = f"/api/chat/{agent_id}/conversations"
        conv_id = c.post(base, body={"user_id": _USER})["id"]

        read = c.get(f"{base}/{conv_id}")
        foreign_read, _ = c.request_raw("GET", f"/api/chat/agent.not-the-owner/conversations/{conv_id}")

        send = f"/api/chat/{agent_id}/send"
        blank_send, _ = c.request_raw("POST", send, body={"conversation_id": conv_id, "content": "   "})
        unknown_send, _ = c.request_raw(
            "POST", send, body={"conversation_id": "no-such-conversation", "content": "hello"}
        )
        foreign_send, _ = c.request_raw(
            "POST",
            "/api/chat/agent.not-the-owner/send",
            body={"conversation_id": conv_id, "content": "hello"},
        )
        return {
            "conv_id": conv_id,
            "read": read,
            "foreign_read": foreign_read,
            "blank_send": blank_send,
            "unknown_send": unknown_send,
            "foreign_send": foreign_send,
        }

    def assert_success(self, ctx: RunContext, result):
        read = result["read"]
        assert read["conversation"]["id"] == result["conv_id"], f"read the wrong conversation: {read!r}"
        assert read["messages"] == [] and read["has_more"] is False, f"a new conversation holds {read!r}"
        assert result["foreign_read"] == 404, f"another agent's path read it: HTTP {result['foreign_read']}"
        assert result["blank_send"] == 400, f"an empty message was accepted: HTTP {result['blank_send']}"
        assert result["unknown_send"] == 404, f"sent into no conversation: HTTP {result['unknown_send']}"
        assert result["foreign_send"] == 404, f"another agent's path sent into it: HTTP {result['foreign_send']}"

    def teardown(self, ctx: RunContext):
        _drop_agent(ctx, "conversations_panel_agent")
