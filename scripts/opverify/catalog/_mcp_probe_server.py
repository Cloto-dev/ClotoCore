"""Hermetic stdio MCP server used by the ``mcp.lifecycle`` operation.

It is registered against a running kernel via ``POST /api/mcp/servers`` with
``command="python3"``; the kernel resolves that bare name to its own
mcp-equipped venv (``resolve_python_command`` →
``data/mcp-servers/.venv/bin/python3``), so this file only needs the ``mcp``
SDK that venv already carries — no system-python dependency.

It exposes a single trivial tool, ``ping`` → ``"pong"``, enough to prove the
full register → connect → tool-discovery → call → stop → reap path. Speaks the
modern low-level ``Server`` API (3-arg ``app.run`` with initialization
options), which matches the SDK version the kernel venv pins.
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

app = Server("opverify-lifecycle-probe")


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
    async with stdio_server() as (read, write):
        await app.run(read, write, app.create_initialization_options())


if __name__ == "__main__":
    asyncio.run(main())
