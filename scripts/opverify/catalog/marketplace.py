"""Marketplace domain — the connector catalog surface.

``catalog`` proves the kernel can fetch and render the remote hub registry
(a network read against the live marketplace).

``install_cycle`` drives the three mutating routes (install → batch-install →
uninstall). It is emphatically not phase 0: it needs the network, the hub, and
the install engine.

It also needs care that the rest of the catalog does not. The kernel's
``data_dir`` is anchored to the *binary*, not to the throwaway run dir (see
``deploy/local.py``), so a marketplace install lands in the real
``data/mcp-servers/`` of whatever checkout built the binary, and uninstall
removes that directory. The operation therefore only ever touches a connector
the machine does **not** already have — it picks its target from the catalog's
own ``installed`` flag (which reports both the DB row and the on-disk
directory) and refuses to run rather than pick an installed one. Install then
uninstall of a previously-absent connector returns the machine to where it
started; picking an installed one would delete something the operator owns.
"""

from __future__ import annotations

import time

from . import Operation, RunContext, register


@register
class MarketplaceCatalog(Operation):
    domain = "marketplace"
    name = "catalog"
    covers = ["GET /api/marketplace/catalog"]
    phase0 = False  # network read against the live hub

    def drive(self, ctx: RunContext):
        return ctx.client.get("/api/marketplace/catalog", timeout=30.0)

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result, dict), f"catalog not an object: {result!r}"
        servers = result.get("servers")
        assert isinstance(servers, list), f"catalog missing servers[] list: {result!r}"
        assert servers, "catalog returned zero servers (hub unreachable/empty?)"
        # each entry should at least identify itself.
        for s in servers[:5]:
            assert isinstance(s, dict) and s.get("id"), (
                f"catalog entry missing id: {s!r}"
            )


def _entries(catalog):
    return (catalog or {}).get("servers") or []


def _entry(catalog, server_id):
    return next((e for e in _entries(catalog) if e.get("id") == server_id), None)


@register
class MarketplaceInstallCycle(Operation):
    """Install a connector, prove it landed, then remove it and prove it is gone.

    Both install routes are asynchronous — they answer ``{started: true}`` and
    do the work in a background task — so success is *not* the 200. It is the
    catalog reporting the connector as installed afterwards, at the version the
    registry advertises, and then reporting it absent again after the
    uninstall. ``batch-install`` is driven against the same (now removed)
    connector so the second half also proves the uninstall really cleared the
    on-disk directory: a batch install of an id the kernel still considered
    present would be skipped instead of performed.

    ``auto_start`` is false throughout: this operation is about vendoring, and
    starting the connector would spawn a child process whose reaping is
    ``mcp.lifecycle``'s subject, not this one's.
    """

    domain = "marketplace"
    name = "install_cycle"
    covers = [
        "POST /api/marketplace/install",
        "POST /api/marketplace/batch-install",
        "DELETE /api/marketplace/servers/{id}",
    ]
    phase0 = False

    install_timeout = 300.0

    def _pick(self, catalog):
        """The smallest connector this machine does not already have.

        "Smallest" = fewest declared dependencies, then fewest required env
        vars, so the operation costs as little install time as possible and
        does not need credentials it has no way to supply.
        """
        candidates = [
            e
            for e in _entries(catalog)
            if not e.get("installed")
            and not e.get("running")
            and e.get("id")
            and e.get("directory")
        ]
        candidates.sort(
            key=lambda e: (
                len(e.get("dependencies") or []),
                len(e.get("env_vars") or []),
                e["id"],
            )
        )
        return candidates[0] if candidates else None

    def _wait_installed(self, ctx, server_id, want: bool):
        deadline = time.monotonic() + self.install_timeout
        last = None
        while time.monotonic() < deadline:
            catalog = ctx.client.get(
                "/api/marketplace/catalog", timeout=60.0
            )
            last = _entry(catalog, server_id)
            if last is not None and bool(last.get("installed")) is want:
                return last
            time.sleep(2.0)
        return last

    def drive(self, ctx: RunContext):
        c = ctx.client
        catalog = c.get("/api/marketplace/catalog", timeout=60.0)
        target = self._pick(catalog)
        if target is None:
            return {"target": None, "candidates": len(_entries(catalog))}
        server_id = target["id"]
        ctx.scratch["marketplace_installed"] = server_id

        c.post(
            "/api/marketplace/install",
            body={"server_id": server_id, "auto_start": False},
            timeout=60.0,
        )
        after_install = self._wait_installed(ctx, server_id, want=True)

        c.delete(f"/api/marketplace/servers/{server_id}", timeout=120.0)
        after_uninstall = self._wait_installed(ctx, server_id, want=False)

        c.post(
            "/api/marketplace/batch-install",
            body={"server_ids": [server_id], "auto_start": False},
            timeout=60.0,
        )
        after_batch = self._wait_installed(ctx, server_id, want=True)

        c.delete(f"/api/marketplace/servers/{server_id}", timeout=120.0)
        final = self._wait_installed(ctx, server_id, want=False)
        ctx.scratch.pop("marketplace_installed", None)

        return {
            "target": server_id,
            "registry_version": target.get("version"),
            "before": target,
            "after_install": after_install,
            "after_uninstall": after_uninstall,
            "after_batch": after_batch,
            "final": final,
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["target"] is not None, (
            f"no uninstalled connector in the catalog "
            f"({result.get('candidates')} entries) — this operation refuses to "
            f"install over a connector the machine already has, because the "
            f"uninstall half would then delete the operator's own files"
        )
        sid = result["target"]
        assert result["before"].get("installed") is False, (
            f"picked an already-installed connector {sid}: {result['before']!r}"
        )

        after = result["after_install"]
        assert after is not None and after.get("installed") is True, (
            f"{sid} is not installed after POST /install: {after!r}"
        )
        assert after.get("installed_version") == result["registry_version"], (
            f"{sid} installed at {after.get('installed_version')!r}, registry "
            f"advertises {result['registry_version']!r}"
        )
        assert after.get("update_available") is False, (
            f"a freshly installed {sid} already reports an update available: "
            f"{after!r}"
        )

        assert (result["after_uninstall"] or {}).get("installed") is False, (
            f"{sid} survived the uninstall: {result['after_uninstall']!r} "
            f"(the DB row or the on-disk directory is still there)"
        )
        assert (result["after_batch"] or {}).get("installed") is True, (
            f"batch-install did not install {sid}: {result['after_batch']!r}"
        )
        assert (result["final"] or {}).get("installed") is False, (
            f"{sid} survived the second uninstall: {result['final']!r}"
        )

    def teardown(self, ctx: RunContext):
        # Only ever removes a connector this operation installed.
        server_id = ctx.scratch.pop("marketplace_installed", None)
        if server_id:
            try:
                ctx.client.delete(
                    f"/api/marketplace/servers/{server_id}", timeout=120.0
                )
            except Exception:  # noqa: BLE001
                pass
