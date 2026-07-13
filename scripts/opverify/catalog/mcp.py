"""MCP domain.

``list`` is a phase-0 read check (a fresh isolated instance registers zero
runnable servers, but the list route must return a well-formed
``{servers, count}`` shape). The full register → start → tool-discovery →
call → stop → **reap (orphan 0)** lifecycle — the operation that exercises
the OS-dependent subprocess-reaping bug class (Goal #145) — needs the Python
MCP venv and is added in phase 1 (``mcp.lifecycle``).
"""

from __future__ import annotations

from . import Operation, RunContext, register


_AGENT = "agent.cloto_default"


@register
class McpList(Operation):
    domain = "mcp"
    name = "list"
    covers = ["GET /api/mcp/servers"]
    phase0 = True

    def drive(self, ctx: RunContext):
        return ctx.client.get("/api/mcp/servers")

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result, dict), f"mcp servers payload not an object: {result!r}"
        assert isinstance(result.get("servers"), list), (
            f"servers[] missing/not a list: {result!r}"
        )
        assert "count" in result, f"count missing: {result!r}"


@register
class McpAccess(Operation):
    domain = "mcp"
    name = "access"
    covers = ["GET /api/mcp/access/by-agent/{agent_id}"]
    phase0 = False

    def drive(self, ctx: RunContext):
        return ctx.client.get(f"/api/mcp/access/by-agent/{_AGENT}")

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result, dict), f"access payload not an object: {result!r}"
        assert result.get("agent_id") == _AGENT, (
            f"access payload agent_id mismatch: {result!r}"
        )
        assert isinstance(result.get("entries"), list), (
            f"access entries[] missing/not a list: {result!r}"
        )
