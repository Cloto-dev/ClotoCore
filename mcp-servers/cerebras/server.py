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

import httpx
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

# ============================================================
# Configuration (from environment variables)
# ============================================================

# API key is managed by kernel LLM proxy (MGP §13.4).
PROVIDER_ID = os.environ.get("CEREBRAS_PROVIDER", "cerebras")
MODEL_ID = os.environ.get("CEREBRAS_MODEL", "llama3.1-70b")
API_URL = os.environ.get(
    "CEREBRAS_API_URL", "http://127.0.0.1:8082/v1/chat/completions"
)
REQUEST_TIMEOUT = int(os.environ.get("CEREBRAS_TIMEOUT_SECS", "120"))

# ============================================================
# LLM Utilities (ported from crates/shared/src/llm.rs)
# ============================================================


def build_system_prompt(agent: dict) -> str:
    """Build the system prompt for an Cloto agent."""
    name = agent.get("name", "Agent")
    description = agent.get("description", "")
    metadata = agent.get("metadata", {})

    has_memory = bool(metadata.get("preferred_memory", ""))
    memory_line = (
        "You have persistent memory — you can recall past conversations with your operator.\n"
        if has_memory
        else ""
    )

    return (
        f"You are {name}, an AI agent running on the Cloto platform.\n"
        f"Cloto is a local, self-hosted AI container system — all data stays on your "
        f"operator's hardware and is never sent to any external service.\n"
        f"{memory_line}"
        f"You can extend your capabilities at runtime using the create_mcp_server tool "
        f"to build new Python-based MCP tools when your current toolset is insufficient.\n"
        f"\n"
        f"IMPORTANT: You must never fabricate or hallucinate information about your "
        f"own capabilities, connected servers, or available tools. If you are unsure "
        f"about what you can do, say so honestly. Only describe capabilities you have "
        f"actually been provided with.\n"
        f"\n"
        f"{description}"
    )


def build_chat_messages(
    agent: dict, message: dict, context: list[dict]
) -> list[dict]:
    """Build the standard OpenAI-compatible messages array."""
    messages = [{"role": "system", "content": build_system_prompt(agent)}]

    for msg in context:
        source = msg.get("source", {})
        # Handle both serde internally-tagged {"type": "User", ...}
        # and legacy externally-tagged {"User": {...}} formats
        src_type = source.get("type", "") if isinstance(source, dict) else ""
        if src_type in ("User",) or "User" in source or "user" in source:
            role = "user"
        elif src_type in ("Agent",) or "Agent" in source or "agent" in source:
            role = "assistant"
        else:
            role = "system"
        messages.append({"role": role, "content": msg.get("content", "")})

    messages.append({"role": "user", "content": message.get("content", "")})
    return messages


def parse_chat_content(response_data: dict) -> str:
    """Extract text content from a chat completions response."""
    # Standard OpenAI error format
    if "error" in response_data:
        error = response_data["error"]
        msg = error.get("message", str(error)) if isinstance(error, dict) else str(error)
        raise ValueError(f"Cerebras API Error: {msg}")

    # Cerebras non-standard error format
    if response_data.get("type", "").endswith("error"):
        msg = response_data.get("message", "Unknown error")
        raise ValueError(f"Cerebras API Error: {msg}")

    try:
        return response_data["choices"][0]["message"]["content"]
    except (KeyError, IndexError, TypeError) as e:
        raise ValueError(
            f"Invalid Cerebras API response: missing choices[0].message.content: {e}"
        ) from e


async def call_cerebras_api(messages: list[dict]) -> dict:
    """Send a request via the kernel LLM proxy (MGP §13.4).

    Note: Cerebras does not support tool schemas, so no tools parameter.
    """
    body: dict = {
        "model": MODEL_ID,
        "messages": messages,
        "stream": False,
    }

    async with httpx.AsyncClient(timeout=REQUEST_TIMEOUT) as client:
        response = await client.post(
            API_URL,
            json=body,
            headers={
                "X-LLM-Provider": PROVIDER_ID,
                "Content-Type": "application/json",
            },
        )
        response.raise_for_status()
        return response.json()


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
            inputSchema={
                "type": "object",
                "properties": {
                    "agent": {
                        "type": "object",
                        "description": "Agent metadata (name, description, metadata)",
                    },
                    "message": {
                        "type": "object",
                        "description": "User message with 'content' field",
                    },
                    "context": {
                        "type": "array",
                        "description": "Conversation context messages",
                        "items": {"type": "object"},
                    },
                },
                "required": ["agent", "message", "context"],
            },
        ),
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "think":
        return await handle_think(arguments)
    else:
        return [
            TextContent(
                type="text",
                text=json.dumps({"error": f"Unknown tool: {name}"}),
            )
        ]


async def handle_think(arguments: dict) -> list[TextContent]:
    """Handle 'think' tool: simple text generation."""
    try:
        agent = arguments.get("agent", {})
        message = arguments.get("message", {})
        context = arguments.get("context", [])

        messages = build_chat_messages(agent, message, context)
        response_data = await call_cerebras_api(messages)
        content = parse_chat_content(response_data)

        return [
            TextContent(
                type="text", text=json.dumps({"type": "final", "content": content})
            )
        ]
    except Exception as e:
        return [
            TextContent(
                type="text", text=json.dumps({"error": str(e)})
            )
        ]


async def main():
    async with stdio_server() as (read_stream, write_stream):
        await server.run(
            read_stream, write_stream, server.create_initialization_options()
        )


if __name__ == "__main__":
    asyncio.run(main())
