"""System domain — the always-on introspection surface (version + metrics).

Read-only, no LLM; kept out of the phase-0 spine only because it is not part
of the minimal boot-proof subset.
"""

from __future__ import annotations

from . import Operation, RunContext, register


@register
class SystemInfo(Operation):
    domain = "system"
    name = "info"
    covers = ["GET /api/system/version", "GET /api/metrics"]
    phase0 = False

    def drive(self, ctx: RunContext):
        version = ctx.client.get("/api/system/version")
        metrics = ctx.client.get("/api/metrics")
        return {"version": version, "metrics": metrics}

    def assert_success(self, ctx: RunContext, result):
        v = result["version"]
        # version may be a bare string or an object carrying a version field.
        ok_v = isinstance(v, str) and v or (
            isinstance(v, dict) and any(
                isinstance(v.get(k), str) and v.get(k)
                for k in ("version", "server_version")
            )
        )
        assert ok_v, f"version read returned nothing usable: {v!r}"
        assert result["metrics"] is not None, "metrics read returned nothing"
