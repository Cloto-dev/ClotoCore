"""
Cloto MCP Server: Cerebras
Ultra-high-speed OpenAI-compatible reasoning engine via MCP protocol.
Ported from plugins/cerebras/src/lib.rs + crates/shared/src/llm.rs

gpt-oss-120b supports tool calling via the OpenAI-compatible API.
"""

import asyncio
import json
import os
import sys

# Resolve parent directory for common module import.
# Handle Windows UNC paths (\\?\...) that Python may receive from the kernel.
_script_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.normpath(os.path.join(_script_dir, "..")))

from common.llm_provider import (
    ProviderConfig,
    THINK_INPUT_SCHEMA,
    THINK_WITH_TOOLS_INPUT_SCHEMA,
    handle_think,
    handle_think_with_tools,
    model_supports_tools,
    run_server,
)
from mcp.server import Server
from mcp.types import TextContent, Tool

# ============================================================
# Configuration (from environment variables)
# ============================================================

# API key is managed by kernel LLM proxy (MGP §13.4).
config = ProviderConfig(
    provider_id=os.environ.get("CEREBRAS_PROVIDER", "cerebras"),
    model_id=os.environ.get("CEREBRAS_MODEL", "gpt-oss-120b"),
    api_url=os.environ.get(
        "CEREBRAS_API_URL", "http://127.0.0.1:8082/v1/chat/completions"
    ),
    request_timeout=int(os.environ.get("CEREBRAS_TIMEOUT_SECS", "120")),
    supports_tools=True,
    display_name="Cerebras",
)

# ============================================================
# MCP Server
# ============================================================

server = Server("cloto-mcp-cerebras")


@server.list_tools()
async def list_tools() -> list[Tool]:
    tools = [
        Tool(
            name="think",
            description=(
                "Generate a text response using Cerebras LLM. "
                "Ultra-high-speed inference."
            ),
            inputSchema=THINK_INPUT_SCHEMA,
        ),
    ]

    if model_supports_tools(config):
        tools.append(
            Tool(
                name="think_with_tools",
                description=(
                    "Generate a response that may include tool calls. "
                    "Returns either final text or a list of tool calls to execute."
                ),
                inputSchema=THINK_WITH_TOOLS_INPUT_SCHEMA,
            )
        )

    return tools


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "think":
        return await handle_think(config, arguments)
    elif name == "think_with_tools":
        return await handle_think_with_tools(config, arguments)
    else:
        return [
            TextContent(
                type="text",
                text=json.dumps({"error": f"Unknown tool: {name}"}),
            )
        ]


if __name__ == "__main__":
    asyncio.run(run_server(server))
