"""Shared helpers for the operations that stand up the hermetic stdio MCP
probe server (``_mcp_probe_server.py``) to drive a kernel surface that only
exists when *some* MCP server is registered.

Three surfaces need one: the MCP settings/access/lifecycle routes (they key
off a registered server row), the memory routes (every one of them dispatches
through the Memory capability, so with no memory server the kernel answers
400 / an empty fallback and nothing can be asserted), and the recall-precision
knob (same dispatcher, feature-detected).

Registration is by absolute path with a bare ``command="python3"``, which the
kernel resolves to its own mcp-equipped venv (``resolve_python_command``) —
the same trick ``mcp.lifecycle`` uses, so no system ``mcp`` install is needed.

``teardown_probe`` stops **before** it deletes on purpose: ``stop_server`` is
what clears the capability dispatcher (``dispatcher.remove_server``), while
``DELETE /api/mcp/servers/{name}`` goes through ``disconnect_server``, which
does not. Deleting a memory probe without stopping it first leaves the kernel
resolving the Memory capability to a server that no longer exists, and the
next operation's memory reads answer from the error fallback instead.
"""

from __future__ import annotations

import os
import time

PROBE_SERVER = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "_mcp_probe_server.py"
)

# Every tool the probe advertises in --memory mode. The kernel's
# `classify_tool` maps each of these names onto the Memory capability, so
# registering the probe makes it the memory server for the run.
MEMORY_TOOLS = (
    "list_memories",
    "import_memories",
    "update_memory",
    "delete_memory",
    "list_episodes",
    "delete_episode",
    "get_recall_precision",
    "set_recall_precision",
)


def register_probe(
    client,
    name: str,
    extra_args=(),
    description: str = "opverify probe server",
    connect_timeout: float = 20.0,
):
    """Register the probe under ``name`` and block until it reports
    ``Connected``. Returns ``(registration_payload, server_row)``; the row is
    ``None`` if it never connected (the caller asserts on that).

    Any stale row from an aborted previous run is removed first.
    """
    try:
        client.post(f"/api/mcp/servers/{name}/stop", timeout=15.0)
    except Exception:  # noqa: BLE001 - nothing to stop is the normal case
        pass
    try:
        client.delete(f"/api/mcp/servers/{name}", timeout=15.0)
    except Exception:  # noqa: BLE001
        pass

    reg = client.post(
        "/api/mcp/servers",
        body={
            "name": name,
            "command": "python3",
            "args": [PROBE_SERVER, *extra_args],
            "description": description,
        },
        timeout=90.0,
    )
    return reg, wait_connected(client, name, timeout=connect_timeout)


def wait_connected(client, name: str, timeout: float = 20.0):
    """Poll ``GET /api/mcp/servers`` until ``name`` reports ``Connected``.
    Returns the server row, or the last row seen (or None) on timeout."""
    deadline = time.monotonic() + timeout
    row = None
    while time.monotonic() < deadline:
        servers = client.get("/api/mcp/servers")
        row = next(
            (s for s in servers.get("servers", []) if s.get("id") == name), None
        )
        if row and row.get("status") == "Connected":
            return row
        time.sleep(0.4)
    return row


def teardown_probe(client, name: str) -> None:
    """Stop (clears the capability dispatcher) then delete (drops the DB row).
    Best-effort: never raises."""
    try:
        client.post(f"/api/mcp/servers/{name}/stop", timeout=20.0)
    except Exception:  # noqa: BLE001
        pass
    try:
        client.delete(f"/api/mcp/servers/{name}", timeout=20.0)
    except Exception:  # noqa: BLE001
        pass
