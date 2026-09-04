"""MCP domain.

``list`` is a phase-0 read check (a fresh isolated instance registers zero
runnable servers, but the list route must return a well-formed
``{servers, count}`` shape).

``lifecycle`` drives the full register → connect → tool-discovery → call →
stop → **reap (orphan 0)** path — the operation that exercises the
OS-dependent subprocess-reaping bug class. It registers a
hermetic single-tool stdio server (``_mcp_probe_server.py``) via bare
``command="python3"``, which the kernel resolves to its own mcp-equipped venv
(``resolve_python_command``), so no system ``mcp`` install is needed. Success
= the server reaches ``Connected`` with a discovered tool, the tool call
returns its payload (the admin ``/mcp/call`` coordinator path runs as
``Caller::System`` and so is not permission-gated), and — critically — every
child process the server spawned is gone after ``stop`` (proving the Unix
process-group reap in ``mcp_transport::start``; the Windows Job-Object
equivalent is the Phase-3 VM-tier target).
"""

from __future__ import annotations

import os
import subprocess
import time

from . import Operation, RunContext, register
from ._probe import register_probe, teardown_probe, wait_connected

# The hermetic probe server shipped alongside this module; registered by
# absolute path so the kernel's entry-point existence check passes.
_PROBE_SERVER = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "_mcp_probe_server.py"
)
_PROBE_MARKER = "_mcp_probe_server.py"
_LIFECYCLE_NAME = "opverify-lifecycle-probe"


_AGENT = "agent.cloto_default"


@register
class McpList(Operation):
    domain = "mcp"
    name = "list"
    covers = ["GET /api/mcp/servers"]
    phase0 = True

    def drive(self, ctx: RunContext):
        return ctx.client.get("/api/mcp/servers")

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result, dict), (
            f"mcp servers payload not an object: {result!r}"
        )
        assert isinstance(result.get("servers"), list), (
            f"servers[] missing/not a list: {result!r}"
        )
        assert "count" in result, f"count missing: {result!r}"


@register
class McpAccess(Operation):
    domain = "mcp"
    name = "access"
    covers = ["GET /api/mcp/access/by-agent/{agent_id}"]
    # Pure read against a table that is empty on a fresh instance; the shape
    # (agent_id + entries[]) is what the dashboard binds to, and it is
    # assertable with no server registered.
    phase0 = True

    def drive(self, ctx: RunContext):
        return ctx.client.get(f"/api/mcp/access/by-agent/{_AGENT}")

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result, dict), f"access payload not an object: {result!r}"
        assert result.get("agent_id") == _AGENT, (
            f"access payload agent_id mismatch: {result!r}"
        )
        assert isinstance(result.get("entries"), list), (
            f"access entries[] missing/not a list: {result!r}"
        )


def _ps_command(pid: int) -> str:
    try:
        out = subprocess.run(
            ["ps", "-o", "command=", "-p", str(pid)],
            capture_output=True,
            text=True,
            timeout=5.0,
        )
        return out.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


def _pid_alive(pid: int) -> bool:
    try:
        return (
            subprocess.run(
                ["ps", "-p", str(pid)], capture_output=True, timeout=5.0
            ).returncode
            == 0
        )
    except (OSError, subprocess.SubprocessError):
        return False


def _probe_child_pids(daemon_pid: int) -> set:
    """The registered probe server's own descendant pids, matched by the probe
    script marker so transient boot-time children never pollute the set."""
    from .. import oracle as orc

    return {
        p for p in orc._descendant_pids(daemon_pid) if _PROBE_MARKER in _ps_command(p)
    }


@register
class McpLifecycle(Operation):
    domain = "mcp"
    name = "lifecycle"
    covers = [
        "POST /api/mcp/servers",
        "POST /api/mcp/call",
        "POST /api/mcp/servers/{name}/stop",
        "DELETE /api/mcp/servers/{name}",
    ]
    phase0 = False

    def drive(self, ctx: RunContext):
        c = ctx.client
        daemon_pid = ctx.target.pid
        name = _LIFECYCLE_NAME

        # Clean any stale row from a previous aborted run (best-effort).
        try:
            c.delete(f"/api/mcp/servers/{name}", timeout=15.0)
        except Exception:  # noqa: BLE001
            pass

        # 1) register — bare python3 resolves to the kernel's mcp-equipped venv.
        reg = c.post(
            "/api/mcp/servers",
            body={
                "name": name,
                "command": "python3",
                "args": [_PROBE_SERVER],
                "description": "opverify lifecycle probe (single ping tool)",
            },
            timeout=90.0,
        )

        # 2) wait for Connected (the kernel discovers tools on connect).
        row = None
        for _ in range(30):
            servers = c.get("/api/mcp/servers")
            row = next(
                (s for s in servers.get("servers", []) if s.get("id") == name),
                None,
            )
            if row and row.get("status") == "Connected":
                break
            time.sleep(0.5)

        my_pids = _probe_child_pids(daemon_pid) if daemon_pid else set()

        # 3) call the tool (System coordinator path — not permission-gated).
        call_result = None
        call_error = None
        try:
            call_result = c.post(
                "/api/mcp/call",
                body={"server_id": name, "tool_name": "ping", "arguments": {}},
                timeout=30.0,
            )
        except Exception as e:  # noqa: BLE001
            call_error = str(e)

        # 4) stop — the reap point: the process group must be signalled as a
        #    unit, leaving no orphaned child.
        stop_error = None
        try:
            c.post(f"/api/mcp/servers/{name}/stop", timeout=20.0)
        except Exception as e:  # noqa: BLE001
            stop_error = str(e)

        leaked = set(my_pids)
        for _ in range(20):
            leaked = {p for p in my_pids if _pid_alive(p)}
            if not leaked:
                break
            time.sleep(0.5)

        # 5) delete — remove the DB row so the isolated instance ends clean.
        try:
            c.delete(f"/api/mcp/servers/{name}", timeout=20.0)
        except Exception:  # noqa: BLE001
            pass

        return {
            "reg_tools": reg.get("tools") if isinstance(reg, dict) else None,
            "status": row.get("status") if row else None,
            "status_message": row.get("status_message") if row else None,
            "row_tools": row.get("tools") if row else None,
            "spawned_pids": sorted(my_pids),
            "leaked_pids": sorted(leaked),
            "call_result": call_result,
            "call_error": call_error,
            "stop_error": stop_error,
            "pid_introspection": daemon_pid is not None,
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["status"] == "Connected", (
            f"probe server did not reach Connected: status={result['status']!r} "
            f"msg={result['status_message']!r} (is the kernel MCP venv present "
            f"with the mcp SDK?)"
        )
        assert result["reg_tools"] and "ping" in result["reg_tools"], (
            f"registration did not discover the ping tool: {result['reg_tools']!r}"
        )
        assert result["call_error"] is None, f"tool call failed: {result['call_error']}"
        cr = result["call_result"]
        text = ""
        if isinstance(cr, dict):
            assert cr.get("isError") is not True, f"tool call returned isError: {cr!r}"
            for part in cr.get("content", []) or []:
                if isinstance(part, dict) and part.get("type") == "text":
                    text += part.get("text", "")
        assert "pong" in text, f"tool call did not return pong: {cr!r}"

        # Reap assertion — only meaningful where we can introspect pids.
        if result["pid_introspection"]:
            assert result["spawned_pids"], (
                "no child process observed for the registered server — cannot "
                "verify reap (did the server spawn?)"
            )
            assert not result["leaked_pids"], (
                f"orphaned child processes survived stop: {result['leaked_pids']} "
                f"(process-group reap regression)"
            )

    def teardown(self, ctx: RunContext):
        # Safety net if drive() raised before its own delete.
        try:
            ctx.client.delete(f"/api/mcp/servers/{_LIFECYCLE_NAME}", timeout=15.0)
        except Exception:  # noqa: BLE001
            pass


_SETTINGS_NAME = "opverify-settings-probe"
_GRANT_AGENT = "agent.cloto_default"


def _entry_keys(access_payload):
    """(entry_type, agent_id, tool_name) for each entry in a server access
    payload — enough to tell a grant apart without depending on row ids."""
    entries = (access_payload or {}).get("entries") or []
    return {
        (e.get("entry_type"), e.get("agent_id"), e.get("tool_name"))
        for e in entries
        if isinstance(e, dict)
    }


@register
class McpServerSettings(Operation):
    """The per-server settings / access-control / lifecycle surface.

    These five routes all key off a *registered* server row, so the operation
    registers the hermetic probe and drives them against it:

    * ``GET``/``PUT settings`` — flip ``default_policy`` between the only two
      legal values and read it back through ``GET``; then assert an illegal
      value is refused and does not change the stored one.
    * ``GET``/``PUT access``   — replace the access entry set and read it back.
      Two invariants of the handler are asserted rather than assumed: an entry
      whose ``server_id`` disagrees with the path is refused (otherwise the
      route would be a way to write grants for a *different* server), and a
      ``capability`` entry is refused from the bulk path.
    * ``POST restart`` / ``POST stop`` + ``POST start`` — the server comes back
      to ``Connected`` and its tool is callable again afterwards. "Restarted"
      is not a status message; it is a server that answers.

    ``start`` is deliberately driven from a *stopped* server: the manager
    refuses to start one that is already ``Connected``, so a start that is
    asserted while running would be asserting an error path.

    Not phase 0 (needs the kernel's mcp-equipped venv), like ``mcp.lifecycle``.
    """

    domain = "mcp"
    name = "server_settings"
    covers = [
        "GET /api/mcp/servers/{name}/settings",
        "PUT /api/mcp/servers/{name}/settings",
        "GET /api/mcp/servers/{name}/access",
        "PUT /api/mcp/servers/{name}/access",
        "POST /api/mcp/servers/{name}/restart",
        "POST /api/mcp/servers/{name}/start",
    ]
    phase0 = False

    def drive(self, ctx: RunContext):
        c = ctx.client
        name = _SETTINGS_NAME
        ctx.scratch["settings_probe"] = name
        _reg, row = register_probe(
            c, name, description="opverify settings/lifecycle probe"
        )
        if (row or {}).get("status") != "Connected":
            return {"status": (row or {}).get("status")}

        # -- settings -------------------------------------------------------
        initial = c.get(f"/api/mcp/servers/{name}/settings")
        booted_policy = (initial or {}).get("default_policy")
        flipped = "opt-out" if booted_policy != "opt-out" else "opt-in"
        c.put(
            f"/api/mcp/servers/{name}/settings",
            body={"default_policy": flipped},
        )
        after_policy = c.get(f"/api/mcp/servers/{name}/settings")
        bad_policy_status, _ = c.request_raw(
            "PUT",
            f"/api/mcp/servers/{name}/settings",
            body={"default_policy": "opverify-not-a-policy"},
        )
        after_bad_policy = c.get(f"/api/mcp/servers/{name}/settings")

        # -- access ---------------------------------------------------------
        access_before = c.get(f"/api/mcp/servers/{name}/access")
        grant = {
            "entry_type": "server_grant",
            "agent_id": _GRANT_AGENT,
            "server_id": name,
            "tool_name": None,
            "permission": "allow",
            "granted_by": "opverify",
            "granted_at": "2026-01-01T00:00:00Z",
        }
        put_access = c.put(
            f"/api/mcp/servers/{name}/access", body={"entries": [grant]}
        )
        access_after = c.get(f"/api/mcp/servers/{name}/access")

        foreign = dict(grant, server_id="opverify-some-other-server")
        foreign_status, foreign_body = c.request_raw(
            "PUT", f"/api/mcp/servers/{name}/access", body={"entries": [foreign]}
        )
        capability = dict(grant, entry_type="capability")
        capability_status, _ = c.request_raw(
            "PUT",
            f"/api/mcp/servers/{name}/access",
            body={"entries": [capability]},
        )
        access_after_refusals = c.get(f"/api/mcp/servers/{name}/access")

        # -- lifecycle ------------------------------------------------------
        c.post(f"/api/mcp/servers/{name}/restart", timeout=60.0)
        after_restart = wait_connected(c, name, timeout=30.0)
        call_after_restart = c.post(
            "/api/mcp/call",
            body={"server_id": name, "tool_name": "ping", "arguments": {}},
            timeout=30.0,
        )

        c.post(f"/api/mcp/servers/{name}/stop", timeout=30.0)
        stopped = _row(c, name)
        c.post(f"/api/mcp/servers/{name}/start", timeout=60.0)
        after_start = wait_connected(c, name, timeout=30.0)
        call_after_start = c.post(
            "/api/mcp/call",
            body={"server_id": name, "tool_name": "ping", "arguments": {}},
            timeout=30.0,
        )

        # Drop the grant we wrote so nothing outlives the operation.
        c.put(f"/api/mcp/servers/{name}/access", body={"entries": []})

        return {
            "status": "Connected",
            "booted_policy": booted_policy,
            "flipped": flipped,
            "after_policy": after_policy,
            "bad_policy_status": bad_policy_status,
            "after_bad_policy": after_bad_policy,
            "access_before": _entry_keys(access_before),
            "put_access": put_access,
            "access_after": _entry_keys(access_after),
            "foreign_status": foreign_status,
            "foreign_body": foreign_body,
            "capability_status": capability_status,
            "access_after_refusals": _entry_keys(access_after_refusals),
            "after_restart": (after_restart or {}).get("status"),
            "call_after_restart": _text(call_after_restart),
            "stopped": (stopped or {}).get("status"),
            "after_start": (after_start or {}).get("status"),
            "call_after_start": _text(call_after_start),
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["status"] == "Connected", (
            f"settings probe did not connect: {result['status']!r}"
        )

        assert result["booted_policy"] in ("opt-in", "opt-out"), (
            f"settings read returned no usable default_policy: "
            f"{result['booted_policy']!r}"
        )
        assert (result["after_policy"] or {}).get("default_policy") == result[
            "flipped"
        ], (
            f"default_policy did not persist: "
            f"{(result['after_policy'] or {}).get('default_policy')!r} != "
            f"{result['flipped']!r}"
        )
        assert result["after_policy"].get("command") == "python3", (
            f"settings lost the registered command: {result['after_policy']!r}"
        )
        assert result["bad_policy_status"] == 400, (
            f"an unknown default_policy was accepted: HTTP "
            f"{result['bad_policy_status']}"
        )
        assert (result["after_bad_policy"] or {}).get("default_policy") == result[
            "flipped"
        ], (
            f"the refused settings write changed the policy anyway: "
            f"{result['after_bad_policy']!r}"
        )

        assert result["access_before"] == set(), (
            f"the freshly registered server already had access entries: "
            f"{result['access_before']}"
        )
        assert (result["put_access"] or {}).get("count") == 1, (
            f"access PUT reported {result['put_access']!r}, wanted count=1"
        )
        assert ("server_grant", _GRANT_AGENT, None) in result["access_after"], (
            f"the written grant is not readable back: {result['access_after']}"
        )
        assert result["foreign_status"] == 400, (
            f"an entry naming a different server_id was accepted: HTTP "
            f"{result['foreign_status']} {result['foreign_body'][:200]} — this "
            f"route would then be a way to write another server's grants"
        )
        assert result["capability_status"] == 400, (
            f"a capability entry was accepted through the bulk path: HTTP "
            f"{result['capability_status']}"
        )
        assert result["access_after_refusals"] == result["access_after"], (
            f"a refused access PUT still changed the entries: "
            f"{result['access_after_refusals']} != {result['access_after']}"
        )

        assert result["after_restart"] == "Connected", (
            f"the server did not come back after restart: "
            f"{result['after_restart']!r}"
        )
        assert "pong" in result["call_after_restart"], (
            f"the restarted server does not answer its tool: "
            f"{result['call_after_restart']!r}"
        )
        assert result["stopped"] != "Connected", (
            f"stop left the server Connected: {result['stopped']!r} — the "
            f"start below would then be asserting an error path"
        )
        assert result["after_start"] == "Connected", (
            f"the server did not come back after start: "
            f"{result['after_start']!r}"
        )
        assert "pong" in result["call_after_start"], (
            f"the started server does not answer its tool: "
            f"{result['call_after_start']!r}"
        )

    def teardown(self, ctx: RunContext):
        name = ctx.scratch.pop("settings_probe", None)
        if not name:
            return
        try:
            ctx.client.put(f"/api/mcp/servers/{name}/access", body={"entries": []})
        except Exception:  # noqa: BLE001
            pass
        teardown_probe(ctx.client, name)


def _row(client, name):
    servers = client.get("/api/mcp/servers")
    return next((s for s in servers.get("servers", []) if s.get("id") == name), None)


def _text(call_result) -> str:
    """Concatenated text content of a /mcp/call result."""
    if not isinstance(call_result, dict):
        return ""
    out = ""
    for part in call_result.get("content", []) or []:
        if isinstance(part, dict) and part.get("type") == "text":
            out += part.get("text", "")
    return out
