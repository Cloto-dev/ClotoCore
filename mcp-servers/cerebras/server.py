"""
Cloto MCP Server: Cerebras
Ultra-high-speed OpenAI-compatible reasoning engine via MCP protocol.
Ported from plugins/cerebras/src/lib.rs + crates/shared/src/llm.rs

NOTE: Cerebras API rejects JSON schema grammar in tool definitions,
so this server only exposes `think` (no `think_with_tools`).
"""

import asyncio
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from common.llm_provider import (
    ProviderConfig,
    THINK_INPUT_SCHEMA,
    handle_think,
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
    model_id=os.environ.get("CEREBRAS_MODEL", "llama3.1-70b"),
    api_url=os.environ.get(
        "CEREBRAS_API_URL", "http://127.0.0.1:8082/v1/chat/completions"
    ),
    request_timeout=int(os.environ.get("CEREBRAS_TIMEOUT_SECS", "120")),
    supports_tools=False,
    display_name="Cerebras",
)

# ============================================================
# MCP Server
# ============================================================

server = Server("cloto-mcp-cerebras")


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="think",
            description=(
                "Generate a text response using Cerebras LLM. "
                "Ultra-high-speed inference. No tool-calling support."
            ),
            inputSchema=THINK_INPUT_SCHEMA,
        ),
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "think":
        return await handle_think(config, arguments)
    else:
        return [
            TextContent(
                type="text",
                text=json.dumps({"error": f"Unknown tool: {name}"}),
            )
        ]


if __name__ == "__main__":
    asyncio.run(run_server(server))
