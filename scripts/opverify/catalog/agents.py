"""Agents domain — full create → observe → power → delete lifecycle.

Success = every stage is externally observable: the created agent appears in
the list, power toggle reports the new state, and after delete it is gone.
No LLM is involved (power toggle only flips a flag), so this is a phase-0
spine operation. Field-name gotchas: create uses ``default_engine``; power
uses ``enabled``; the list/response echo ``default_engine_id``.
"""

from __future__ import annotations

from . import Operation, RunContext, register

_TEST_NAME = "opverify-lifecycle-agent"


def _find(agents, agent_id):
    return next((a for a in agents if a.get("id") == agent_id), None)


@register
class AgentsList(Operation):
    domain = "agents"
    name = "list"
    covers = ["GET /api/agents"]
    phase0 = True

    def drive(self, ctx: RunContext):
        return ctx.client.get("/api/agents")

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result, list), f"agents list not an array: {result!r}"
        assert result, "agents list empty (expected seeded default agent)"
        for a in result:
            assert "id" in a and "enabled" in a, f"agent missing fields: {a!r}"


@register
class AgentsLifecycle(Operation):
    domain = "agents"
    name = "lifecycle"
    covers = [
        "POST /api/agents",
        "POST /api/agents/{id}/power",
        "DELETE /api/agents/{id}",
    ]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        # 1. create
        created = c.post(
            "/api/agents",
            body={
                "name": _TEST_NAME,
                "description": "opverify lifecycle probe agent",
                "default_engine": "cerebras",
            },
        )
        agent_id = created["id"]
        ctx.scratch["lifecycle_agent_id"] = agent_id

        # 2. appears in list
        listed = c.get("/api/agents")
        present = _find(listed, agent_id) is not None

        # 3. power on / off
        on = c.post(f"/api/agents/{agent_id}/power", body={"enabled": True})
        off = c.post(f"/api/agents/{agent_id}/power", body={"enabled": False})

        # 4. delete
        c.delete(f"/api/agents/{agent_id}")
        after = c.get("/api/agents")
        gone = _find(after, agent_id) is None
        ctx.scratch.pop("lifecycle_agent_id", None)

        return {
            "agent_id": agent_id,
            "present_after_create": present,
            "power_on": on,
            "power_off": off,
            "gone_after_delete": gone,
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["agent_id"], "no agent id returned from create"
        assert result["present_after_create"], "created agent not found in list"
        assert result["power_on"].get("enabled") is True, (
            f"power-on did not report enabled=true: {result['power_on']!r}"
        )
        assert result["power_off"].get("enabled") is False, (
            f"power-off did not report enabled=false: {result['power_off']!r}"
        )
        assert result["gone_after_delete"], "agent still present after delete"

    def teardown(self, ctx: RunContext):
        agent_id = ctx.scratch.pop("lifecycle_agent_id", None)
        if agent_id:
            try:
                ctx.client.delete(f"/api/agents/{agent_id}")
            except Exception:
                pass
