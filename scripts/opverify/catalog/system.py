"""System domain — the always-on introspection surface (version + metrics),
the read-only half of the uninstall flow (the Danger Zone's first gate), and
the admin-key rotation.

Read-only, no LLM; kept out of the phase-0 spine only because it is not part
of the minimal boot-proof subset.

``rotate_key`` is the one operation in the catalog that changes the credential
the harness itself is using, so it declares ``order = 100`` and runs after
everything else in its slice. See its docstring for why the *other* key route
(``POST /api/system/invalidate-key``) is not driven here.
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
    # Pure read of always-on introspection; nothing external is required.
    phase0 = True

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


@register
class SystemRotateKey(Operation):
    """Rotate the admin API key, then keep using the instance with the new one.

    ``POST /api/system/regenerate-key`` mints a key, persists it, and swaps it
    into the live auth state with no restart. That makes it the only operation
    here that invalidates the harness's own credential mid-run, so it declares
    ``order = 100`` and runs last within its slice, and it repairs the harness
    on the way out: ``ctx.client`` **and** ``ctx.target`` are moved onto the new
    key, so the deployment's teardown (``POST /api/system/shutdown``, the only
    way this kernel exits cleanly — it installs no SIGTERM handler) still
    authenticates.

    Success is asserted from both sides of the swap, because either alone is
    passable by a handler that did half the job: the new key must authenticate
    a subsequent request, and the old key must now be refused. A regenerate
    that returned a key without activating it would pass the second check; one
    that activated a key without invalidating the old would pass the first.

    ``POST /api/system/invalidate-key`` is deliberately **not** driven. It
    revokes the key presented in the request, and ``check_auth`` accepts
    exactly one key (the live ``admin_api_key``, minus the revoked set) — so
    the only key it can ever revoke is the one the caller needs, and there is
    no second credential and no unauthenticated route with which to mint a
    replacement. Driving it would end the run's ability to talk to the daemon
    at all, including the authenticated shutdown that teardown depends on and
    that the MCP-orphan and DB-corruption oracles are measured after. It is
    drivable only where a restart can follow — the VM tiers, where a snapshot
    absorbs it.
    """

    domain = "system"
    name = "rotate_key"
    covers = ["POST /api/system/regenerate-key"]
    phase0 = True
    # Runs after every other operation in the slice: it replaces the admin key
    # that all of them (and teardown) authenticate with.
    order = 100

    def drive(self, ctx: RunContext):
        c = ctx.client
        old_key = c.api_key

        rotated = c.post("/api/system/regenerate-key", timeout=30.0)
        new_key = (rotated or {}).get("api_key")
        assert new_key, f"regenerate returned no api_key: {rotated!r}"

        # Move the harness onto the new key *before* anything else — from here
        # on the old one is dead, including for teardown.
        c.api_key = new_key
        ctx.target.api_key = new_key
        ctx.log(f"system.rotate_key: admin key rotated (len={len(new_key)})")

        new_key_status, _ = c.request_raw("GET", "/api/agents")

        # The old key must now be refused. Use a throwaway client so ctx.client
        # keeps the working credential even if this raises.
        from ..client import ClotoClient

        stale = ClotoClient(c.base_url, old_key)
        old_key_status, _ = stale.request_raw("GET", "/api/agents")

        return {
            "rotated": rotated,
            "new_key_len": len(new_key),
            "same_as_old": new_key == old_key,
            "new_key_status": new_key_status,
            "old_key_status": old_key_status,
        }

    def assert_success(self, ctx: RunContext, result):
        assert not result["same_as_old"], "regenerate returned the current key"
        assert result["new_key_len"] == 64, (
            f"regenerated key is {result['new_key_len']} chars, wanted 64"
        )
        assert (result["rotated"] or {}).get("persisted_to"), (
            f"regenerate did not report where it persisted the key: "
            f"{result['rotated']!r}"
        )
        assert result["new_key_status"] == 200, (
            f"the regenerated key does not authenticate: HTTP "
            f"{result['new_key_status']} — it was minted but not activated"
        )
        assert result["old_key_status"] == 403, (
            f"the previous key still authenticates: HTTP "
            f"{result['old_key_status']}, wanted 403"
        )
