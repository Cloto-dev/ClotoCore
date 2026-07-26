"""System domain — the always-on introspection surface (version + metrics),
plus the read-only half of the uninstall flow (the Danger Zone's first gate).

Read-only, no LLM; kept out of the phase-0 spine only because it is not part
of the minimal boot-proof subset.
"""

from __future__ import annotations

from . import Operation, RunContext, register

# Serialized tier names, narrowest first — the plan's `tier` field and every
# entry's effective tier come from this enum (DEFENDER_DESIGN.md §7).
_TIERS = ["application", "user_data", "assets", "everything"]


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
        ok_v = (
            isinstance(v, str)
            and v
            or (
                isinstance(v, dict)
                and any(
                    isinstance(v.get(k), str) and v.get(k)
                    for k in ("version", "server_version")
                )
            )
        )
        assert ok_v, f"version read returned nothing usable: {v!r}"
        assert result["metrics"] is not None, "metrics read returned nothing"


@register
class SystemUninstallPlan(Operation):
    """Enumerate what an uninstall would remove, at the narrowest and the
    widest scope, without removing anything.

    This is the readable half of the uninstall flow: `POST
    /api/system/uninstall` cannot run in the local tier (it would remove the
    harness machine's own installation) and is covered by the VM-tier scenario
    instead, but the enumeration behind the Danger Zone's first gate is pure
    read and belongs here — a plan that stops naming the installation is a
    real regression, and nothing else in the catalog would notice it.

    Asserted properties, all of them load-bearing invariants of §7 rather than
    incidental shape:

    * tier 1 is the default — omitting the parameter must produce the same
      scope as asking for it explicitly, because the destructive endpoint
      shares that default;
    * an entry never sits above the requested scope (containment: a directory
      carries the widest tier of anything inside it);
    * a wider scope is a superset — tier 4 names everything tier 1 does;
    * the notes survive to the surface (they are the honest statement of what
      the enumeration cannot promise).
    """

    domain = "system"
    name = "uninstall_plan"
    covers = ["GET /api/system/uninstall/plan"]
    phase0 = True

    def drive(self, ctx: RunContext):
        default = ctx.client.get("/api/system/uninstall/plan", timeout=60.0)
        narrow = ctx.client.get(
            "/api/system/uninstall/plan", params={"tier": "1"}, timeout=60.0
        )
        widest = ctx.client.get(
            "/api/system/uninstall/plan", params={"tier": "4"}, timeout=60.0
        )
        return {"default": default, "narrow": narrow, "widest": widest}

    def assert_success(self, ctx: RunContext, result):
        for label, payload in result.items():
            assert isinstance(payload, dict), f"{label}: response not an object"
            plan = payload.get("plan")
            summary = payload.get("summary")
            assert isinstance(plan, dict), f"{label}: missing plan object: {payload!r}"
            assert isinstance(summary, dict), f"{label}: missing summary: {payload!r}"
            assert plan.get("plan_version") == 1, (
                f"{label}: unexpected plan_version: {plan.get('plan_version')!r}"
            )
            assert isinstance(plan.get("entries"), list), f"{label}: entries not a list"
            assert isinstance(plan.get("skipped"), list), f"{label}: skipped not a list"
            assert plan.get("notes"), (
                f"{label}: plan carries no notes; §7 requires them verbatim"
            )
            assert plan.get("data_dir"), f"{label}: plan does not name its data_dir"
            assert summary.get("entries") == len(plan["entries"]), (
                f"{label}: summary entry count disagrees with the plan"
            )

        narrow, widest = result["narrow"]["plan"], result["widest"]["plan"]
        assert narrow["tier"] == "application", f"tier=1 resolved to {narrow['tier']!r}"
        assert widest["tier"] == "everything", f"tier=4 resolved to {widest['tier']!r}"
        assert result["default"]["plan"]["tier"] == "application", (
            "omitting tier did not default to the narrowest scope"
        )

        # Containment: nothing in a plan is removed at a wider scope than the
        # one that was asked for.
        for label, plan in (("narrow", narrow), ("widest", widest)):
            ceiling = _TIERS.index(plan["tier"])
            for entry in plan["entries"]:
                tier = entry.get("tier")
                assert tier in _TIERS, f"{label}: unknown entry tier {tier!r}"
                assert _TIERS.index(tier) <= ceiling, (
                    f"{label}: entry {entry.get('id')!r} is tier {tier!r}, "
                    f"above the requested {plan['tier']!r}"
                )

        # Cumulative scope: what tier 1 removes, tier 4 also removes. Compared
        # by id — a path can move between tiers by containment (a tier-1 file
        # collapsing into the tier-4 container that holds it), so comparing
        # paths would report that as a loss.
        missing = {e["id"] for e in narrow["entries"]} - {
            e["id"] for e in widest["entries"]
        }
        covered = {s["id"] for s in widest["skipped"] if s.get("reason") == "covered_by_parent"}
        assert not (missing - covered), (
            f"tier 4 does not cover tier 1: {sorted(missing - covered)}"
        )
