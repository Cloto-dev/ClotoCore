"""
Cloto MCP Server: Claude (Anthropic)
First-class Anthropic Messages API integration via MCP protocol.

Unlike OpenAI-compatible providers (Cerebras, DeepSeek), Anthropic uses
a fundamentally different API format: separate system parameter, content
blocks for tool_use, and x-api-key authentication. This server handles
all format conversion natively.
"""

import asyncio
import json
import logging
import os
import sys

import httpx
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

# Resolve parent directory for common module import.
_script_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.normpath(os.path.join(_script_dir, "..")))

from common.llm_provider import build_system_prompt

logger = logging.getLogger(__name__)

# ============================================================
# Configuration
# ============================================================

PROVIDER_ID = os.environ.get("CLAUDE_PROVIDER", "claude")
MODEL_ID = os.environ.get("CLAUDE_MODEL", "claude-sonnet-4-6")
API_URL = os.environ.get(
    "CLAUDE_API_URL", "http://127.0.0.1:8082/v1/messages"
)
TIMEOUT_SECS = int(os.environ.get("CLAUDE_TIMEOUT_SECS", "120"))
MAX_TOKENS = int(os.environ.get("CLAUDE_MAX_TOKENS", "4096"))

# ============================================================
# Anthropic API Adapter
# ============================================================


def _extract_system_and_messages(
    agent: dict, message: dict, context: list[dict], tools: list[dict] | None = None
) -> tuple[str, list[dict]]:
    """Build Anthropic-format system prompt and messages array.

    Anthropic requires the system prompt as a top-level parameter,
    NOT as a message with role=system. We reuse the common
    build_system_prompt() for consistency, then build messages separately.
    """
    system = build_system_prompt(agent, tools)

    messages: list[dict] = []
    for msg in context:
        source = msg.get("source", {})
        src_type = source.get("type", "") if isinstance(source, dict) else ""
        if src_type in ("User",) or "User" in source or "user" in source:
            role = "user"
        elif src_type in ("Agent",) or "Agent" in source or "agent" in source:
            role = "assistant"
        else:
            continue  # Skip system messages (already in system param)
        content = msg.get("content", "")
        if content:
            messages.append({"role": role, "content": content})

    messages.append({"role": "user", "content": message.get("content", "")})
    return system, messages


def _convert_tools_to_anthropic(openai_tools: list[dict]) -> list[dict]:
    """Convert OpenAI function-calling tool schemas to Anthropic format.

    OpenAI:   {"type": "function", "function": {"name", "description", "parameters"}}
    Anthropic: {"name", "description", "input_schema"}
    """
    anthropic_tools = []
    for tool in openai_tools:
        fn = tool.get("function", {})
        name = fn.get("name", "")
        if not name:
            continue
        anthropic_tools.append({
            "name": name,
            "description": fn.get("description", ""),
            "input_schema": fn.get("parameters", {"type": "object", "properties": {}}),
        })
    return anthropic_tools


def _convert_tool_history(tool_history: list[dict]) -> list[dict]:
    """Convert OpenAI-format tool history to Anthropic message format.

    OpenAI tool history contains:
      - {"role": "assistant", "tool_calls": [...]}
      - {"role": "tool", "tool_call_id": "...", "content": "..."}

    Anthropic expects:
      - {"role": "assistant", "content": [{"type": "tool_use", "id", "name", "input"}]}
      - {"role": "user", "content": [{"type": "tool_result", "tool_use_id", "content"}]}
    """
    messages: list[dict] = []
    for entry in tool_history:
        role = entry.get("role", "")

        if role == "assistant":
            # Convert tool_calls to Anthropic content blocks
            tool_calls = entry.get("tool_calls", [])
            content_blocks: list[dict] = []
            text = entry.get("content", "")
            if text:
                content_blocks.append({"type": "text", "text": text})
            for tc in tool_calls:
                fn = tc.get("function", {})
                args = fn.get("arguments", "{}")
                if isinstance(args, str):
                    try:
                        args = json.loads(args)
                    except json.JSONDecodeError:
                        args = {}
                content_blocks.append({
                    "type": "tool_use",
                    "id": tc.get("id", ""),
                    "name": fn.get("name", ""),
                    "input": args,
                })
            if content_blocks:
                messages.append({"role": "assistant", "content": content_blocks})

        elif role == "tool":
            # Convert to Anthropic tool_result
            messages.append({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": entry.get("tool_call_id", ""),
                    "content": entry.get("content", ""),
                }],
            })

    return messages


def _parse_anthropic_response(response_data: dict) -> dict:
    """Parse Anthropic Messages API response into a ThinkResult.

    Returns either:
      {"type": "final", "content": "..."}
    or:
      {"type": "tool_calls", "assistant_content": "...", "calls": [...]}
    """
    # Error handling
    if "error" in response_data:
        err = response_data["error"]
        msg = err.get("message", str(err)) if isinstance(err, dict) else str(err)
        raise ValueError(f"Claude API Error: {msg}")

    content_blocks = response_data.get("content", [])
    stop_reason = response_data.get("stop_reason", "end_turn")

    # Extract text and tool_use blocks
    text_parts: list[str] = []
    tool_calls: list[dict] = []

    thinking_parts: list[str] = []

    for block in content_blocks:
        block_type = block.get("type", "")
        if block_type == "text":
            text_parts.append(block.get("text", ""))
        elif block_type == "thinking":
            thinking_parts.append(block.get("thinking", ""))
        elif block_type == "tool_use":
            tool_calls.append({
                "id": block.get("id", ""),
                "name": block.get("name", ""),
                "arguments": block.get("input", {}),
            })

    # Tool calls detected
    if stop_reason == "tool_use" and tool_calls:
        # Prefer text content, fall back to thinking content (extended thinking models)
        assistant_content = "\n".join(text_parts) if text_parts else (
            "\n".join(thinking_parts) if thinking_parts else None
        )
        return {
            "type": "tool_calls",
            "assistant_content": assistant_content,
            "calls": tool_calls,
        }

    # Final text response
    return {
        "type": "final",
        "content": "\n".join(text_parts) if text_parts else "",
    }


# ============================================================
# API Call
# ============================================================


class ClaudeApiError(Exception):
    def __init__(self, message: str, code: str = "unknown", status_code: int = 0):
        super().__init__(message)
        self.message = message
        self.code = code
        self.status_code = status_code


async def call_claude_api(
    system: str,
    messages: list[dict],
    tools: list[dict] | None = None,
) -> dict:
    """Send a request to Claude via the kernel LLM proxy."""
    body: dict = {
        "model": MODEL_ID,
        "system": system,
        "messages": messages,
        "max_tokens": MAX_TOKENS,
    }

    if tools:
        body["tools"] = tools

    try:
        async with httpx.AsyncClient(timeout=TIMEOUT_SECS) as client:
            response = await client.post(
                API_URL,
                json=body,
                headers={
                    "X-LLM-Provider": PROVIDER_ID,
                    "Content-Type": "application/json",
                },
            )
    except httpx.ConnectError:
        raise ClaudeApiError(
            "Cannot connect to LLM proxy. Ensure the kernel is running.",
            "connection_failed",
        )
    except httpx.TimeoutException:
        raise ClaudeApiError(
            f"Claude request timed out after {TIMEOUT_SECS}s.",
            "timeout",
        )

    if response.status_code >= 400:
        try:
            err_body = response.json()
            err_obj = err_body.get("error", {})
            msg = err_obj.get("message", f"HTTP {response.status_code}")
            code = err_obj.get("code", "unknown")
        except Exception:
            msg = f"HTTP {response.status_code}"
            code = "unknown"
        raise ClaudeApiError(msg, code, response.status_code)

    return response.json()


# ============================================================
# Error Helper
# ============================================================


def _error_response(error: Exception) -> list[TextContent]:
    if isinstance(error, ClaudeApiError):
        return [TextContent(type="text", text=json.dumps({
            "error": error.message, "error_code": error.code,
        }))]
    return [TextContent(type="text", text=json.dumps({
        "error": str(error), "error_code": "internal",
    }))]


# ============================================================
# MCP Tool Handlers
# ============================================================


async def handle_think(arguments: dict) -> list[TextContent]:
    """Handle 'think' tool: simple text generation (no tools)."""
    try:
        agent = arguments.get("agent", {})
        message = arguments.get("message", {})
        context = arguments.get("context", [])

        system, messages = _extract_system_and_messages(agent, message, context)
        response_data = await call_claude_api(system, messages)

        # Extract text from content blocks
        content_blocks = response_data.get("content", [])
        text = "\n".join(
            b.get("text", "") for b in content_blocks if b.get("type") == "text"
        )
        return [TextContent(
            type="text", text=json.dumps({"type": "final", "content": text})
        )]
    except Exception as e:
        return _error_response(e)


async def handle_think_with_tools(arguments: dict) -> list[TextContent]:
    """Handle 'think_with_tools' tool: may return tool calls or final text."""
    try:
        agent = arguments.get("agent", {})
        message = arguments.get("message", {})
        context = arguments.get("context", [])
        tools = arguments.get("tools", [])
        tool_history = arguments.get("tool_history", [])

        system, messages = _extract_system_and_messages(
            agent, message, context, tools=tools
        )

        # Append tool history (converted to Anthropic format)
        messages.extend(_convert_tool_history(tool_history))

        # Convert OpenAI tool schemas to Anthropic format
        anthropic_tools = _convert_tools_to_anthropic(tools) if tools else None

        response_data = await call_claude_api(system, messages, anthropic_tools)
        result = _parse_anthropic_response(response_data)

        return [TextContent(type="text", text=json.dumps(result))]
    except Exception as e:
        return _error_response(e)


# ============================================================
# MCP Server Definition
# ============================================================

THINK_INPUT_SCHEMA = {
    "type": "object",
    "properties": {
        "agent": {"type": "object", "description": "Agent metadata"},
        "message": {"type": "object", "description": "User message with 'content' field"},
        "context": {"type": "array", "description": "Conversation context messages"},
    },
    "required": ["agent", "message", "context"],
}

THINK_WITH_TOOLS_INPUT_SCHEMA = {
    "type": "object",
    "properties": {
        "agent": {"type": "object"},
        "message": {"type": "object"},
        "context": {"type": "array"},
        "tools": {"type": "array", "description": "Available tool schemas (OpenAI format)"},
        "tool_history": {"type": "array", "description": "Prior tool calls and results"},
    },
    "required": ["agent", "message", "context", "tools", "tool_history"],
}

server = Server("cloto-mcp-claude")


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="think",
            description="Generate a text response using Claude. High-quality reasoning with safety.",
            inputSchema=THINK_INPUT_SCHEMA,
        ),
        Tool(
            name="think_with_tools",
            description=(
                "Generate a response that may include tool calls. "
                "Returns either final text or a list of tool calls to execute."
            ),
            inputSchema=THINK_WITH_TOOLS_INPUT_SCHEMA,
        ),
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "think":
        return await handle_think(arguments)
    elif name == "think_with_tools":
        return await handle_think_with_tools(arguments)
    else:
        return [TextContent(
            type="text", text=json.dumps({"error": f"Unknown tool: {name}"})
        )]


async def main():
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(name)s] %(levelname)s: %(message)s",
    )
    logger.info(f"Starting Claude MCP server (model={MODEL_ID})")
    async with stdio_server() as (read_stream, write_stream):
        await server.run(
            read_stream, write_stream, server.create_initialization_options()
        )


if __name__ == "__main__":
    asyncio.run(main())
