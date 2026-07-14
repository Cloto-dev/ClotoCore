"""Setup domain — the first-run onboarding gate.

``GET /api/setup/status`` is what the dashboard polls to decide whether to
route a fresh launch through the setup wizard or straight into the app.
"Success" here means the running kernel reports itself as a *usable,
completed* setup: a booted instance with its seeded ``agents`` row (or a dev
checkout) must answer ``setup_complete = true`` and not be stuck mid-
bootstrap — otherwise real users get re-prompted through the wizard on every
launch (the bug-384 shape this endpoint's fallback was hardened against).

The SSE ``/api/setup/progress`` stream is intentionally not a catalog target
(see the coverage ignore-list); progress is observed through the wizard flow,
not asserted as a one-shot operation.
"""

from __future__ import annotations

from . import Operation, RunContext, register

_BOOL_FIELDS = (
    "setup_complete",
    "mcp_servers_present",
    "uv_available",
    "venv_exists",
    "setup_in_progress",
)


@register
class SetupStatus(Operation):
    domain = "setup"
    name = "status"
    covers = ["GET /api/setup/status"]

    def drive(self, ctx: RunContext):
        # No auth required on this route (like health), but the client sends
        # the key harmlessly regardless.
        return ctx.client.get("/api/setup/status", timeout=10.0)

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result, dict), f"setup status not an object: {result!r}"
        for field in _BOOL_FIELDS:
            assert isinstance(result.get(field), bool), (
                f"setup status missing/invalid bool '{field}': {result!r}"
            )
        # The running kernel must recognize itself as a usable setup: a fresh
        # boot seeds an agents row (db_has_agents fallback) and a dev checkout
        # reports complete unconditionally.
        assert result["setup_complete"] is True, (
            f"kernel reports setup incomplete: {result!r}"
        )
        # A steady-state instance is not mid-bootstrap.
        assert result["setup_in_progress"] is False, (
            f"setup unexpectedly in progress: {result!r}"
        )
