"""Memory domain.

There is no direct create-memory REST route — memories are produced by the
Memory-capability MCP server (CPersona) via chat/store flows. On a fresh
isolated instance the list is therefore empty, but the *read path* still
returns a well-formed shape (memories[] + count + capabilities), which is a
valid operation-to-success check for phase 0. The lock/unlock/update mutation
operations require a seeded memory (a real memory MCP) and are added in
phase 1.
"""

from __future__ import annotations

from . import Operation, RunContext, register


@register
class MemoryList(Operation):
    domain = "memory"
    name = "list"
    covers = ["GET /api/memories"]
    phase0 = True

    def drive(self, ctx: RunContext):
        return ctx.client.get("/api/memories")

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result, dict), f"memories payload not an object: {result!r}"
        assert isinstance(result.get("memories"), list), (
            f"memories[] missing/not a list: {result!r}"
        )
        assert "count" in result, f"count missing: {result!r}"
        assert isinstance(result.get("capabilities"), dict), (
            f"capabilities missing/not an object: {result!r}"
        )
