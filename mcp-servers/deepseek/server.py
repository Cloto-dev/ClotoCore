"""
Cloto MCP Server: DeepSeek
OpenAI-compatible reasoning engine via MCP protocol.
Ported from plugins/deepseek/src/lib.rs + crates/shared/src/llm.rs
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

API_KEY = os.environ.get("DEEPSEEK_API_KEY", "")
MODEL_ID = os.environ.get("DEEPSEEK_MODEL", "deepseek-chat")
API_URL = os.environ.get(
    "DEEPSEEK_API_URL", "https://api.deepseek.com/chat/completions"
)
REQUEST_TIMEOUT = int(os.environ.get("DEEPSEEK_TIMEOUT_SECS", "60"))

# ============================================================
# LLM Utilities (ported from crates/shared/src/llm.rs)
# ============================================================


def model_supports_tools(model_id: str) -> bool:
    """deepseek-reasoner (R1) explicitly does not support tool schemas."""
    return "reasoner" not in model_id


def build_system_prompt(agent: dict) -> str:
    """Build the system prompt for an Cloto agent.

    Ported from llm::build_system_prompt().
    """
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
        f"{description}"
    )


def build_chat_messages(
    agent: dict, message: dict, context: list[dict]
) -> list[dict]:
    """Build the standard OpenAI-compatible messages array.

    Returns [system_message, ...context_messages, user_message].
    Ported from llm::build_chat_messages().
    """
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
    """Extract text content from a chat completions response.

    Ported from llm::parse_chat_content().
    """
    # Standard OpenAI error format
    if "error" in response_data:
        error = response_data["error"]
        msg = error.get("message", str(error)) if isinstance(error, dict) else str(error)
        raise ValueError(f"DeepSeek API Error: {msg}")

    # Cerebras non-standard error format
    if response_data.get("type", "").endswith("error"):
        msg = response_data.get("message", "Unknown error")
        raise ValueError(f"DeepSeek API Error: {msg}")

    try:
        return response_data["choices"][0]["message"]["content"]
    except (KeyError, IndexError, TypeError) as e:
        raise ValueError(
            f"Invalid DeepSeek API response: missing choices[0].message.content: {e}"
        ) from e


def parse_chat_think_result(response_data: dict) -> dict:
    """Parse a chat completions response into a ThinkResult.

    Returns either:
      {"type": "final", "content": "..."}
    or:
      {"type": "tool_calls", "assistant_content": "...", "calls": [...]}

    Ported from llm::parse_chat_think_result().
    """
    # Standard OpenAI error format
    if "error" in response_data:
        error = response_data["error"]
        msg = error.get("message", str(error)) if isinstance(error, dict) else str(error)
        raise ValueError(f"DeepSeek API Error: {msg}")

    # Cerebras non-standard error format
    if response_data.get("type", "").endswith("error"):
        msg = response_data.get("message", "Unknown error")
        raise ValueError(f"DeepSeek API Error: {msg}")

    try:
        choice = response_data["choices"][0]
    except (KeyError, IndexError, TypeError) as e:
        raise ValueError(f"Invalid API response: missing choices[0]: {e}") from e

    message_obj = choice.get("message", {})
    finish_reason = choice.get("finish_reason", "stop")

    if finish_reason == "tool_calls" or "tool_calls" in message_obj:
        tool_calls_arr = message_obj.get("tool_calls", [])
        calls = []
        for tc in tool_calls_arr:
            tc_id = tc.get("id", "")
            function = tc.get("function", {})
            name = function.get("name", "")
            arguments_str = function.get("arguments", "{}")
            try:
                arguments = json.loads(arguments_str)
            except json.JSONDecodeError:
                arguments = {}

            if tc_id and name:
                calls.append(
                    {"id": tc_id, "name": name, "arguments": arguments}
                )

        if calls:
            return {
                "type": "tool_calls",
                "assistant_content": message_obj.get("content"),
                "calls": calls,
            }

    content = message_obj.get("content", "")
    if content is None:
        content = ""
    return {"type": "final", "content": content}


async def call_deepseek_api(
    messages: list[dict], tools: list[dict] | None = None
) -> dict:
    """Send a request to the DeepSeek API and return the response."""
    if not API_KEY:
        raise ValueError("DeepSeek API Key not configured (DEEPSEEK_API_KEY)")

    body: dict = {
        "model": MODEL_ID,
        "messages": messages,
        "stream": False,
    }

    if tools and model_supports_tools(MODEL_ID):
        body["tools"] = tools

    async with httpx.AsyncClient(timeout=REQUEST_TIMEOUT) as client:
        response = await client.post(
            API_URL,
            json=body,
            headers={
                "Authorization": f"Bearer {API_KEY}",
                "Content-Type": "application/json",
            },
        )
        response.raise_for_status()
        return response.json()


# ============================================================
# MCP Server
# ============================================================

server = Server("cloto-mcp-deepseek")


@server.list_tools()
async def list_tools() -> list[Tool]:
    tools = [
        Tool(
            name="think",
            description=(
                "Generate a text response using DeepSeek LLM. "
                "Use this for simple text generation without tool support."
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

    if model_supports_tools(MODEL_ID):
        tools.append(
            Tool(
                name="think_with_tools",
                description=(
                    "Generate a response that may include tool calls. "
                    "Returns either final text or a list of tool calls to execute."
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
                        "tools": {
                            "type": "array",
                            "description": "Available tool schemas (OpenAI format)",
                            "items": {"type": "object"},
                        },
                        "tool_history": {
                            "type": "array",
                            "description": "Prior tool calls and results",
                            "items": {"type": "object"},
                        },
                    },
                    "required": [
                        "agent",
                        "message",
                        "context",
                        "tools",
                        "tool_history",
                    ],
                },
            )
        )

    return tools


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "think":
        return await handle_think(arguments)
    elif name == "think_with_tools":
        return await handle_think_with_tools(arguments)
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
        response_data = await call_deepseek_api(messages)
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


async def handle_think_with_tools(arguments: dict) -> list[TextContent]:
    """Handle 'think_with_tools' tool: may return tool calls or final text."""
    try:
        agent = arguments.get("agent", {})
        message = arguments.get("message", {})
        context = arguments.get("context", [])
        tools = arguments.get("tools", [])
        tool_history = arguments.get("tool_history", [])

        messages = build_chat_messages(agent, message, context)
        # Append tool history (assistant messages with tool_calls + tool results)
        messages.extend(tool_history)

        response_data = await call_deepseek_api(messages, tools)
        result = parse_chat_think_result(response_data)

        return [TextContent(type="text", text=json.dumps(result))]
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
