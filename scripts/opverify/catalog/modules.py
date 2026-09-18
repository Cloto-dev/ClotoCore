"""Modules domain — runtime UI modules and the panel write gate.

``list`` reads ``GET /api/modules``: a fresh instance has no modules, and the
route must still answer with a list.

``write-gate`` drives the panel write gate against a panel that does not
exist, which is the only panel a fresh instance has. That is not a weaker
check than a successful write: the gate's whole job is to refuse, and a
refusal is what proves the routes are mounted *and* guarded. A successful write
needs a connector installed from the marketplace with a verified seal, which
this tier cannot stage; the kernel's own tests cover that path
(``crates/core/src/handlers/panel_writes.rs``).
"""

from __future__ import annotations

import json

from . import Operation, RunContext, register

_NO_SUCH_PANEL = "opverify-no-such-panel"


@register
class ModulesList(Operation):
    domain = "modules"
    name = "list"
    covers = ["GET /api/modules", "GET /api/modules/write-consents"]
    phase0 = True

    def drive(self, ctx: RunContext):
        return {
            "modules": ctx.client.get("/api/modules"),
            "consents": ctx.client.get("/api/modules/write-consents"),
        }

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result["modules"], list), (
            f"module listing is not a list: {result['modules']!r}"
        )
        assert isinstance(result["consents"], list), (
            f"consent listing is not a list: {result['consents']!r}"
        )


@register
class ModulesWriteGate(Operation):
    domain = "modules"
    name = "write-gate"
    covers = [
        "GET /api/modules/{id}/write-access",
        "PUT /api/modules/{id}/write-consent",
        "DELETE /api/modules/{id}/write-consent",
        "POST /api/modules/{id}/write",
    ]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        base = f"/api/modules/{_NO_SUCH_PANEL}"
        return {
            "access": c.request_raw("GET", f"{base}/write-access"),
            "consent": c.request_raw("PUT", f"{base}/write-consent"),
            "revoke": c.request_raw("DELETE", f"{base}/write-consent"),
            "write": c.request_raw(
                "POST",
                f"{base}/write",
                {
                    "method": "POST",
                    "path": "/api/chat/opverify/messages",
                    "body": {"content": "must not be delivered"},
                },
            ),
        }

    def assert_success(self, ctx: RunContext, result):
        for key in ("access", "consent", "revoke"):
            status, text = result[key]
            assert status == 404, f"{key} on an unknown panel returned {status}: {text}"
        status, text = result["write"]
        assert status == 403, f"a write from an unknown panel returned {status}: {text}"
        error = json.loads(text).get("error", {})
        assert error.get("type") == "PanelWriteDenied", f"unexpected refusal shape: {text}"
