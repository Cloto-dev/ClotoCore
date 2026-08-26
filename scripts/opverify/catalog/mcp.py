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
    phase0 = False

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
