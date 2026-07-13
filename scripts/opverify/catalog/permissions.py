"""Permissions domain — the pending-permission read path plus the
approve/deny decision mutations.

``pending`` (``GET /api/permissions/pending``) is the read check: the route
must return a well-formed list even when empty.

``decide`` exercises the approve/deny mutations against *real* pending
requests. It stages them by registering the shared probe server with
``--declare-perms`` so it advertises MGP-required permissions; the kernel's
MGP Permission Flow (§3) then opens one pending request per permission
(``mgp-<server>-<perm>``, status ``pending``). The op approves one and denies
the other and asserts both leave the pending set — proving the mutations
actually flipped the stored status (not merely returned 200). No live agent /
chat is needed: the admin ``/mcp/call`` path runs as ``Caller::System`` and is
not permission-gated, so a connect-time MGP declaration is the clean way to
manufacture a pending request.
"""

from __future__ import annotations

import os
import time

from . import Operation, RunContext, register

_PROBE_SERVER = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "_mcp_probe_server.py"
)
_PERM_SERVER = "opverify-perm-probe"
_PERM_APPROVE = "opverify.approve-me"
_PERM_DENY = "opverify.deny-me"


def _pending_by_id(client) -> dict:
    rows = client.get("/api/permissions/pending")
    return {r.get("request_id"): r for r in (rows or []) if isinstance(r, dict)}


@register
class PermissionsPending(Operation):
    domain = "permissions"
    name = "pending"
    covers = ["GET /api/permissions/pending"]
    phase0 = False

    def drive(self, ctx: RunContext):
        return ctx.client.get("/api/permissions/pending")

    def assert_success(self, ctx: RunContext, result):
        # accept a bare list or a {requests: [...]} / {pending: [...]} envelope.
        pending = result
        if isinstance(result, dict):
            pending = (
                result.get("requests")
                or result.get("pending")
                or result.get("permissions")
            )
        assert isinstance(pending, list), (
            f"pending permissions read did not return a list: {result!r}"
        )


@register
class PermissionsDecide(Operation):
    domain = "permissions"
    name = "decide"
    covers = [
        "POST /api/permissions/{id}/approve",
        "POST /api/permissions/{id}/deny",
    ]
    phase0 = False

    def drive(self, ctx: RunContext):
        c = ctx.client
        rid_ok = f"mgp-{_PERM_SERVER}-{_PERM_APPROVE}"
        rid_no = f"mgp-{_PERM_SERVER}-{_PERM_DENY}"

        # YOLO mode auto-approves declared permissions instead of raising
        # pending requests (the seeded dev DB ships with yolo_mode=true), which
        # would starve this op of anything to decide. Force it off for the
        # staging window and restore the prior value in teardown.
        try:
            ctx.scratch["perm_pre_yolo"] = bool(
                (c.get("/api/settings/yolo") or {}).get("enabled")
            )
            c.put("/api/settings/yolo", body={"enabled": False})
        except Exception:  # noqa: BLE001
            ctx.scratch.setdefault("perm_pre_yolo", None)

        # Clean any stale row from a previous aborted run.
        try:
            c.delete(f"/api/mcp/servers/{_PERM_SERVER}", timeout=15.0)
        except Exception:  # noqa: BLE001
            pass

        # Stage two pending requests via a connect-time MGP permission
        # declaration (register may or may not surface an error — the pending
        # rows are committed either way; we assert on the rows, not the call).
        try:
            c.post(
                "/api/mcp/servers",
                body={
                    "name": _PERM_SERVER,
                    "command": "python3",
                    "args": [
                        _PROBE_SERVER,
                        "--declare-perms",
                        f"{_PERM_APPROVE},{_PERM_DENY}",
                    ],
                    "description": "opverify permission-decision probe",
                },
                timeout=90.0,
            )
        except Exception:  # noqa: BLE001
            pass

        # Wait for both pending requests to appear.
        before = {}
        for _ in range(30):
            before = _pending_by_id(c)
            if rid_ok in before and rid_no in before:
                break
            time.sleep(0.5)

        # Approve one, deny the other.
        approve_error = None
        deny_error = None
        try:
            c.post(f"/api/permissions/{rid_ok}/approve", body={}, timeout=15.0)
        except Exception as e:  # noqa: BLE001
            approve_error = str(e)
        try:
            c.post(f"/api/permissions/{rid_no}/deny", body={}, timeout=15.0)
        except Exception as e:  # noqa: BLE001
            deny_error = str(e)

        # Re-read pending; both decided requests must be gone.
        after = {}
        for _ in range(20):
            after = _pending_by_id(c)
            if rid_ok not in after and rid_no not in after:
                break
            time.sleep(0.3)

        try:
            c.delete(f"/api/mcp/servers/{_PERM_SERVER}", timeout=15.0)
        except Exception:  # noqa: BLE001
            pass

        return {
            "rid_ok": rid_ok,
            "rid_no": rid_no,
            "staged": rid_ok in before and rid_no in before,
            "before_statuses": {
                rid_ok: before.get(rid_ok, {}).get("status"),
                rid_no: before.get(rid_no, {}).get("status"),
            },
            "approve_error": approve_error,
            "deny_error": deny_error,
            "after_ids": sorted(after),
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["staged"], (
            "MGP permission declaration did not raise both pending requests: "
            f"before_statuses={result['before_statuses']!r} — the connect-time "
            "MGP Permission Flow (§3) did not stage them"
        )
        assert result["approve_error"] is None, (
            f"approve mutation failed: {result['approve_error']}"
        )
        assert result["deny_error"] is None, (
            f"deny mutation failed: {result['deny_error']}"
        )
        assert result["rid_ok"] not in result["after_ids"], (
            f"approved request still pending: {result['rid_ok']} "
            f"(approve did not flip the stored status)"
        )
        assert result["rid_no"] not in result["after_ids"], (
            f"denied request still pending: {result['rid_no']} "
            f"(deny did not flip the stored status)"
        )

    def teardown(self, ctx: RunContext):
        try:
            ctx.client.delete(f"/api/mcp/servers/{_PERM_SERVER}", timeout=15.0)
        except Exception:  # noqa: BLE001
            pass
        # Restore YOLO mode to whatever the instance booted with.
        pre = ctx.scratch.get("perm_pre_yolo")
        if pre is not None:
            try:
                ctx.client.put("/api/settings/yolo", body={"enabled": pre})
            except Exception:  # noqa: BLE001
                pass
