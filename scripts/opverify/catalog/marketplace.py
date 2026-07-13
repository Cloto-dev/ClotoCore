"""Marketplace domain — the connector catalog surface.

``catalog`` proves the kernel can fetch and render the remote hub registry
(a network read against the live marketplace). Install / uninstall mutate the
real on-disk connector set and are deferred to a later slice (they belong with
the VM tiers, where a pristine snapshot absorbs the install).
"""

from __future__ import annotations

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
        assert isinstance(servers, list), (
            f"catalog missing servers[] list: {result!r}"
        )
        assert servers, "catalog returned zero servers (hub unreachable/empty?)"
        # each entry should at least identify itself.
        for s in servers[:5]:
            assert isinstance(s, dict) and s.get("id"), (
                f"catalog entry missing id: {s!r}"
            )
