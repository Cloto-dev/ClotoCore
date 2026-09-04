"""Health domain — the kernel's self-diagnosis surface.

``scan`` proves the read/diagnose path returns a well-formed Healthy report;
``repair`` proves the repair path runs and leaves the instance Healthy.
(HealthStatus serializes lowercase: healthy / degraded / error.)

``live`` covers ``GET /api/system/health``, the unauthenticated liveness probe.
Everything in the harness leans on it — ``client.wait_healthy`` gates the boot
and the liveness oracle polls it between operations — yet no operation claimed
it, so the route the whole run is measured against was the one route the
ratchet could not see.
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


@register
class HealthLive(Operation):
    """The liveness probe, asserted on both of its contracts.

    It answers ``{status: "ok", ...}`` and — unlike every other ``/api`` route
    — it is on the public allowlist, so it answers *without* a key. Both halves
    matter: the authenticated call is what the oracle uses, and the
    unauthenticated one is what an external supervisor (installer, systemd,
    container probe) uses. A regression that quietly put this behind auth would
    make every such probe read the daemon as dead.
    """

    domain = "health"
    name = "live"
    covers = ["GET /api/system/health"]
    phase0 = True

    def drive(self, ctx: RunContext):
        authed = ctx.client.get("/api/system/health", timeout=10.0)
        anon = ctx.client.get("/api/system/health", auth=False, timeout=10.0)
        # Control: the public allowlist must be a property of *this* route,
        # not of an auth layer that is off for the whole API.
        anon_agents_status, _ = ctx.client.request_raw(
            "GET", "/api/agents", body=None, auth=False
        )
        return {"authed": authed, "anon": anon, "anon_agents": anon_agents_status}

    def assert_success(self, ctx: RunContext, result):
        for label in ("authed", "anon"):
            payload = result[label]
            assert isinstance(payload, dict), (
                f"{label} health response is not an object: {payload!r}"
            )
            assert payload.get("status") == "ok", (
                f"{label} health reports status={payload.get('status')!r}"
            )
        assert result["anon"].get("status") == result["authed"].get("status"), (
            "the unauthenticated liveness probe disagrees with the "
            "authenticated one"
        )
        assert result["anon_agents"] == 403, (
            f"an unauthenticated GET /api/agents returned "
            f"{result['anon_agents']}, wanted 403 — the anonymous health pass "
            f"above proves nothing if the whole API is open"
        )
