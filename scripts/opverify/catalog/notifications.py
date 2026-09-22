"""Notifications domain — raise an item from outside the kernel and settle it.

``POST /api/notifications`` is the one way a producer that is not an agent
reaches the reader. The operation raises a ``proposal`` (the kind that counts on
the badge), finds it in the listing under the ``external:`` prefix, reads it,
answers it, and checks the badge went up by one and back down. A second raise
with the same id must write nothing: that is what makes a retry safe.

Counts are compared before and after rather than against zero, because a live
kernel may already be holding items of its own.
"""

from __future__ import annotations

import uuid

from . import Operation, RunContext, register


@register
class NotificationsRaiseAndSettle(Operation):
    domain = "notifications"
    name = "raise_and_settle"
    covers = [
        "POST /api/notifications",
        "GET /api/notifications",
        "GET /api/notifications/summary",
        "POST /api/notifications/{item_id}/read",
        "POST /api/notifications/{item_id}/answer",
    ]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        chosen = f"opverify-{uuid.uuid4().hex[:12]}"
        request = {
            "item_id": chosen,
            "kind": "proposal",
            "severity": "warning",
            "title": "opverify: a job went quiet",
            "body": "Raised by the operation catalog; answering it is part of the run.",
        }

        before = c.get("/api/notifications/summary")["summary"]["waiting"]
        first = c.post("/api/notifications", body=request)
        retry = c.post("/api/notifications", body=request)
        item_id = first["item_id"]
        raised = c.get("/api/notifications/summary")["summary"]["waiting"]

        listed = [
            item
            for item in c.get("/api/notifications?unresolved=true&limit=200")["items"]
            if item.get("item_id") == item_id
        ]
        read = c.post(f"/api/notifications/{item_id}/read")
        c.post(f"/api/notifications/{item_id}/answer", body={"decision": "seen by opverify"})
        after = c.get("/api/notifications/summary")["summary"]["waiting"]

        return {
            "chosen": chosen,
            "item_id": item_id,
            "created": first.get("created"),
            "retry_created": retry.get("created"),
            "listed": listed,
            "read_changed": read.get("changed"),
            "waiting": (before, raised, after),
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["item_id"] == f"external:{result['chosen']}", (
            f"an outside item must live under external:, got {result['item_id']!r}"
        )
        assert result["created"] is True, "the first raise did not write"
        assert result["retry_created"] is False, "a retry with the same id wrote a second item"
        assert len(result["listed"]) == 1, f"listed {len(result['listed'])} times"
        item = result["listed"][0]
        assert item.get("kind") == "proposal" and item.get("agent_id") is None, item
        assert result["read_changed"] is True, "marking it read changed nothing"
        before, raised, after = result["waiting"]
        assert raised == before + 1, f"badge went {before} -> {raised}, want +1"
        assert after == before, f"answering left the badge at {after}, want {before}"
