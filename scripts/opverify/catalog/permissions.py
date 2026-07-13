"""Permissions domain — the pending-permission read path.

``GET /api/permissions/pending`` lists permission requests awaiting an
approve/deny decision. On an isolated instance the list is typically empty,
but the read path must return a well-formed list. The approve/deny mutations
need a live pending request (raised by an untrusted MCP tool call) and are
added with the MCP register→call lifecycle in a later slice.
"""

from __future__ import annotations

from . import Operation, RunContext, register


@register
class PermissionsPending(Operation):
    domain = "permissions"
    name = "pending"
    covers = ["GET /api/permissions/pending"]
    phase0 = False

    def drive(self, ctx: RunContext):
        return ctx.client.get("/api/permissions/pending")

    def assert_success(self, ctx: RunContext, result):
        # accept a bare list or a {requests: [...]} / {pending: [...]} envelope.
        pending = result
        if isinstance(result, dict):
            pending = (
                result.get("requests")
                or result.get("pending")
                or result.get("permissions")
            )
        assert isinstance(pending, list), (
            f"pending permissions read did not return a list: {result!r}"
        )
