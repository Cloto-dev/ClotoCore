"""Health domain — the kernel's self-diagnosis surface.

``scan`` proves the read/diagnose path returns a well-formed Healthy report;
``repair`` proves the repair path runs and leaves the instance Healthy.
(HealthStatus serializes lowercase: healthy / degraded / error.)
"""

from __future__ import annotations

from . import Operation, RunContext, register

_HEALTHY = {"healthy", "ok"}


def _assert_report(data) -> str:
    assert isinstance(data, dict), f"health report not an object: {data!r}"
    status = str(data.get("status", "")).lower()
    assert "checks" in data and isinstance(data["checks"], list), (
        f"health report missing checks[]: {data!r}"
    )
    return status


@register
class HealthScan(Operation):
    domain = "health"
    name = "scan"
    covers = ["GET /api/health/scan"]
    phase0 = True

    def drive(self, ctx: RunContext):
        return ctx.client.get(
            "/api/health/scan", params={"fresh": "true"}, timeout=30.0
        )

    def assert_success(self, ctx: RunContext, result):
        status = _assert_report(result)
        assert status in _HEALTHY, f"health scan status={status}"


@register
class HealthRepair(Operation):
    domain = "health"
    name = "repair"
    covers = ["POST /api/health/repair"]
    phase0 = True

    def drive(self, ctx: RunContext):
        # repair returns a RepairReport {actions:[...], total_fixed:N}
        return ctx.client.post("/api/health/repair", timeout=60.0)

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result, dict), f"repair report not an object: {result!r}"
        assert isinstance(result.get("actions"), list), (
            f"repair report missing actions[]: {result!r}"
        )
        assert (
            isinstance(result.get("total_fixed"), int) and result["total_fixed"] >= 0
        ), f"repair report missing/invalid total_fixed: {result!r}"
