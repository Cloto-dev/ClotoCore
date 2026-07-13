"""Hermetic stdio MCP server used by the ``mcp.lifecycle`` and
``permissions.decide`` operations.

It is registered against a running kernel via ``POST /api/mcp/servers`` with
``command="python3"``; the kernel resolves that bare name to its own
mcp-equipped venv (``resolve_python_command`` →
``data/mcp-servers/.venv/bin/python3``), so this file only needs the ``mcp``
SDK that venv already carries — no system-python dependency.

It exposes a single trivial tool, ``ping`` → ``"pong"``, enough to prove the
full register → connect → tool-discovery → call → stop → reap path. Speaks the
modern low-level ``Server`` API (3-arg ``app.run`` with initialization
options), which matches the SDK version the kernel venv pins.

With ``--declare-perms a,b`` it additionally advertises MGP server
capabilities (``capabilities.experimental.mgp`` — the Python-SDK-compatible
path the kernel reads) declaring the ``permissions`` extension and the given
required permissions. That drives the kernel's MGP Permission Flow (§3): the
kernel opens one pending permission request per permission and then *refuses
the connection* until they are approved — exactly the state the
``permissions.decide`` op needs to exercise the approve/deny mutations. The
declared MGP ``version`` matches the kernel's ``MGP_VERSION`` so negotiation
succeeds.
"""

import asyncio
import os
import sys

# Running this file as a script puts its own directory (``catalog/``) on
# ``sys.path[0]``, where the sibling ``mcp.py`` (the opverify MCP domain
# module) would shadow the real ``mcp`` SDK. Scrub our own dir first so the
# venv's ``mcp`` package resolves, not the sibling module.
_self_dir = os.path.dirname(os.path.abspath(__file__))
sys.path[:] = [p for p in sys.path if os.path.abspath(p or ".") != _self_dir]

import mcp.types as types  # noqa: E402
from mcp.server import Server  # noqa: E402
from mcp.server.stdio import stdio_server  # noqa: E402

# Kept in sync with mcp_mgp::MGP_VERSION; negotiate() requires semver
# compatibility with the kernel's client version for the extension to activate.
_MGP_VERSION = "0.6.0"

app = Server("opverify-lifecycle-probe")


def _declared_perms(argv):
    for i, a in enumerate(argv):
        if a == "--declare-perms" and i + 1 < len(argv):
            return [p for p in argv[i + 1].split(",") if p]
    return []


@app.list_tools()
async def list_tools():
    return [
        types.Tool(
            name="ping",
            description="Return pong.",
            inputSchema={"type": "object", "properties": {}},
        )
    ]


@app.call_tool()
async def call_tool(name, arguments):
    return [types.TextContent(type="text", text="pong")]


async def main():
    perms = _declared_perms(sys.argv)
    experimental = None
    if perms:
        experimental = {
            "mgp": {
                "version": _MGP_VERSION,
                "extensions": ["permissions"],
                "permissions_required": perms,
            }
        }
    opts = app.create_initialization_options(experimental_capabilities=experimental)
    async with stdio_server() as (read, write):
        await app.run(read, write, opts)


if __name__ == "__main__":
    asyncio.run(main())
