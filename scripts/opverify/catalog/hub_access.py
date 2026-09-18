"""Hub access domain — this kernel's access token for restricted connectors.

A fresh instance holds no token, so every route is driven to the answer a
fresh instance must give: no token to show, a malformed token refused before
anything is sent to a hub, nothing to renew, nothing to forget. Binding a real
token needs a hub that issued one, which this tier cannot stage; the kernel's
own tests drive bind and renew against a hub double
(``crates/core/src/managers/hub_access.rs``).

Also checked: the status route is not readable without the admin key.
"""

from __future__ import annotations

import json

from . import Operation, RunContext, register


@register
class HubAccessFreshInstance(Operation):
    domain = "hub-access"
    name = "fresh-instance"
    covers = [
        "GET /api/hub-access",
        "POST /api/hub-access/token",
        "POST /api/hub-access/renew",
        "DELETE /api/hub-access/token",
    ]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        return {
            "status": c.request_raw("GET", "/api/hub-access"),
            "anonymous": c.request_raw("GET", "/api/hub-access", auth=False),
            "set": c.request_raw(
                "POST", "/api/hub-access/token", {"token": "not-an-access-token"}
            ),
            "renew": c.request_raw("POST", "/api/hub-access/renew"),
            "forget": c.request_raw("DELETE", "/api/hub-access/token"),
        }

    def assert_success(self, ctx: RunContext, result):
        status, text = result["status"]
        assert status == 200, f"status route: {status} {text}"
        assert json.loads(text)["data"]["token"] is None, f"a fresh instance has no token: {text}"

        status, text = result["anonymous"]
        assert status in (401, 403), f"status readable without the admin key: {status} {text}"

        status, text = result["set"]
        assert status == 400, f"a malformed token must be refused before any hub call: {status} {text}"

        status, text = result["renew"]
        assert status == 404, f"nothing to renew on a fresh instance: {status} {text}"

        status, text = result["forget"]
        assert status == 200, f"forget: {status} {text}"
        assert json.loads(text)["data"]["removed"] is False, f"nothing was stored: {text}"
