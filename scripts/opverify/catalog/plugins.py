"""Plugins domain — the plugin manifest read path, plus the config and
permission stores the dashboard writes through.

``GET /api/plugins`` returns the list of plugin manifests the kernel loaded.
Read-only well-formedness check.

``config`` and ``permissions`` drive the mutation routes. They differ in what
they can address:

* the **config** store (``plugin_configs``) is keyed on a plugin id with no
  existence check at all — the handler upserts whatever id it is given, and
  the kernel itself uses that (``plugin_id='kernel'``) to persist YOLO mode.
  So a probe id is a legitimate target, and the round trip is real.
* the **permission** store (``plugin_settings.allowed_permissions``) only ever
  updates rows that already exist; grant on a missing row is a silent no-op
  and revoke answers "not granted". The rows that do exist come from the seed
  migration, so the operation drives a seeded id.

``POST /api/plugins/apply`` is deliberately **not** claimed by either
operation — see the note at the bottom of this module.
"""

from __future__ import annotations

from . import Operation, RunContext, register


@register
class PluginsList(Operation):
    domain = "plugins"
    name = "list"
    covers = ["GET /api/plugins"]
    # Pure read, no LLM / network / secrets, well-formed on a fresh empty DB.
    phase0 = True

    def drive(self, ctx: RunContext):
        return ctx.client.get("/api/plugins")

    def assert_success(self, ctx: RunContext, result):
        # accept a bare list or a {plugins: [...]} envelope.
        plugins = result.get("plugins") if isinstance(result, dict) else result
        assert isinstance(plugins, list), (
            f"plugins read did not return a list: {result!r}"
        )


_PROBE_PLUGIN = "opverify-config-probe"
# Seeded by migrations (`20260212000000_final_seeds.sql`). A `plugin_settings`
# row must already exist for grant/revoke to do anything, and nothing in the
# API creates one.
_SEEDED_PLUGIN = "memory.ks22"
_PERMISSION = "NetworkAccess"


@register
class PluginsConfig(Operation):
    """The per-plugin key/value config store.

    Write a key, read it back, overwrite it, read it back again. The overwrite
    matters: the handler is an ``INSERT OR REPLACE``, so a version that dropped
    the replace would still pass a single write-then-read.

    The response also masks secrets on the event it broadcasts but *not* on the
    ``GET`` (config may legitimately carry values the operator needs to see),
    so the read-back is byte-exact.
    """

    domain = "plugins"
    name = "config"
    covers = ["GET /api/plugins/{id}/config", "POST /api/plugins/{id}/config"]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        before = c.get(f"/api/plugins/{_PROBE_PLUGIN}/config")
        c.post(
            f"/api/plugins/{_PROBE_PLUGIN}/config",
            body={"key": "opverify_probe", "value": "first"},
        )
        after_write = c.get(f"/api/plugins/{_PROBE_PLUGIN}/config")
        c.post(
            f"/api/plugins/{_PROBE_PLUGIN}/config",
            body={"key": "opverify_probe", "value": "second"},
        )
        after_overwrite = c.get(f"/api/plugins/{_PROBE_PLUGIN}/config")
        # A second key must coexist rather than replace the first.
        c.post(
            f"/api/plugins/{_PROBE_PLUGIN}/config",
            body={"key": "opverify_other", "value": "kept"},
        )
        after_second_key = c.get(f"/api/plugins/{_PROBE_PLUGIN}/config")
        return {
            "before": before,
            "after_write": after_write,
            "after_overwrite": after_overwrite,
            "after_second_key": after_second_key,
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["before"] == {}, (
            f"probe plugin already had config: {result['before']!r}"
        )
        assert result["after_write"].get("opverify_probe") == "first", (
            f"config write not readable back: {result['after_write']!r}"
        )
        assert result["after_overwrite"].get("opverify_probe") == "second", (
            f"config overwrite did not replace the value: "
            f"{result['after_overwrite']!r}"
        )
        assert result["after_second_key"].get("opverify_probe") == "second", (
            f"writing a second key clobbered the first: "
            f"{result['after_second_key']!r}"
        )
        assert result["after_second_key"].get("opverify_other") == "kept", (
            f"second config key not stored: {result['after_second_key']!r}"
        )


@register
class PluginsPermissions(Operation):
    """Grant → read → revoke → read on a plugin's permission set.

    Both mutations are asserted from the read route, and the revoke is
    asserted twice: the permission is gone from the set, and revoking it a
    second time is refused (the handler's ``rows_affected == 0`` path is what
    distinguishes "removed it" from "there was nothing to remove", and a
    revoke that always answered 200 would hide a no-op).
    """

    domain = "plugins"
    name = "permissions"
    covers = [
        "GET /api/plugins/{id}/permissions",
        "POST /api/plugins/{id}/permissions/grant",
        "DELETE /api/plugins/{id}/permissions",
    ]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        ctx.scratch["plugin_perm"] = (_SEEDED_PLUGIN, _PERMISSION)
        before = c.get(f"/api/plugins/{_SEEDED_PLUGIN}/permissions")
        c.post(
            f"/api/plugins/{_SEEDED_PLUGIN}/permissions/grant",
            body={"permission": _PERMISSION},
        )
        after_grant = c.get(f"/api/plugins/{_SEEDED_PLUGIN}/permissions")
        c.delete(
            f"/api/plugins/{_SEEDED_PLUGIN}/permissions",
            body={"permission": _PERMISSION},
        )
        after_revoke = c.get(f"/api/plugins/{_SEEDED_PLUGIN}/permissions")
        double_revoke_status, _ = c.request_raw(
            "DELETE",
            f"/api/plugins/{_SEEDED_PLUGIN}/permissions",
            body={"permission": _PERMISSION},
        )
        ctx.scratch.pop("plugin_perm", None)
        return {
            "before": before,
            "after_grant": after_grant,
            "after_revoke": after_revoke,
            "double_revoke_status": double_revoke_status,
        }

    def assert_success(self, ctx: RunContext, result):
        before = _perms(result["before"])
        assert result["before"].get("plugin_id") == _SEEDED_PLUGIN, (
            f"permissions read did not echo the plugin id: {result['before']!r}"
        )
        assert _PERMISSION not in before, (
            f"{_SEEDED_PLUGIN} already held {_PERMISSION}: {before} — the "
            f"grant below would prove nothing"
        )
        after_grant = _perms(result["after_grant"])
        assert _PERMISSION in after_grant, (
            f"grant did not take effect: {after_grant}"
        )
        after_revoke = _perms(result["after_revoke"])
        assert _PERMISSION not in after_revoke, (
            f"revoke did not take effect: {after_revoke}"
        )
        assert after_revoke == before, (
            f"revoke did not restore the original set: {after_revoke} != {before}"
        )
        assert result["double_revoke_status"] >= 400, (
            f"revoking a permission that is not held answered HTTP "
            f"{result['double_revoke_status']} — a revoke that always succeeds "
            f"cannot report a no-op"
        )

    def teardown(self, ctx: RunContext):
        pair = ctx.scratch.pop("plugin_perm", None)
        if not pair:
            return
        plugin_id, permission = pair
        try:
            ctx.client.delete(
                f"/api/plugins/{plugin_id}/permissions",
                body={"permission": permission},
            )
        except Exception:  # noqa: BLE001
            pass


def _perms(payload) -> set:
    return set((payload or {}).get("permissions") or [])


# ``POST /api/plugins/apply`` is left uncovered on purpose. It writes
# ``plugin_settings.is_active``, and that column is only ever read back through
# ``list_plugins_with_settings``, which merges it onto the *in-process plugin
# registry* — empty in the shipped kernel, so ``GET /api/plugins`` returns
# ``[]`` and the write has no observable at all. An unknown plugin id and a
# real one produce identical responses. Claiming the route here would put a
# tick in the ratchet for an operation that cannot tell success from a silent
# no-op, which is the one thing this catalog is supposed not to do. It becomes
# drivable the moment either the plugin list surfaces DB-only rows or `apply`
# reports what it changed.
