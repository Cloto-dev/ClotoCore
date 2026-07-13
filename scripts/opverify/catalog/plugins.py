"""Plugins domain — the registered-plugin manifest read path.

``GET /api/plugins`` returns the list of plugin manifests the kernel loaded.
Read-only well-formedness check.
"""

from __future__ import annotations

from . import Operation, RunContext, register


@register
class PluginsList(Operation):
    domain = "plugins"
    name = "list"
    covers = ["GET /api/plugins"]
    phase0 = False

    def drive(self, ctx: RunContext):
        return ctx.client.get("/api/plugins")

    def assert_success(self, ctx: RunContext, result):
        # accept a bare list or a {plugins: [...]} envelope.
        plugins = result.get("plugins") if isinstance(result, dict) else result
        assert isinstance(plugins, list), (
            f"plugins read did not return a list: {result!r}"
        )
