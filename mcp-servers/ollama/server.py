"""
Cloto MCP Server: Ollama
Local LLM inference via Ollama's OpenAI-compatible API.
Supports dynamic model switching and local model discovery.

Tools:
  - think:         Generate a text response using the active Ollama model
  - list_models:   List locally installed Ollama models
  - switch_model:  Change the active model for this session
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

BASE_URL = os.environ.get("OLLAMA_BASE_URL", "http://localhost:11434")
MODEL_ID = os.environ.get("OLLAMA_MODEL", "glm-4.7-flash")
REQUEST_TIMEOUT = int(os.environ.get("OLLAMA_TIMEOUT_SECS", "120"))

# Mutable session state
_active_model = MODEL_ID

# ============================================================
# LLM Utilities (shared pattern with cerebras/deepseek)
# ============================================================


def build_system_prompt(agent: dict) -> str:
    """Build the system prompt for a Cloto agent."""
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
    """Build the standard OpenAI-compatible messages array."""
    messages = [{"role": "system", "content": build_system_prompt(agent)}]

    for msg in context:
        source = msg.get("source", {})
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
    """Extract text content from Ollama /api/chat response.

    Ollama native format: { "message": { "role": "assistant", "content": "..." }, ... }
    OpenAI compat format: { "choices": [{ "message": { "content": "..." } }] }
    """
    if "error" in response_data:
        error = response_data["error"]
        msg = error.get("message", str(error)) if isinstance(error, dict) else str(error)
        raise ValueError(f"Ollama API Error: {msg}")

    # Ollama native format
    if "message" in response_data:
        return response_data["message"].get("content", "")

    # OpenAI compat fallback
    try:
        return response_data["choices"][0]["message"]["content"]
    except (KeyError, IndexError, TypeError) as e:
        raise ValueError(
            f"Invalid Ollama API response: {e}"
        ) from e


# ============================================================
# Ollama API
# ============================================================


async def call_ollama_api(messages: list[dict]) -> dict:
    """Send a request to the Ollama native chat API (/api/chat)."""
    body: dict = {
        "model": _active_model,
        "messages": messages,
        "stream": False,
    }

    async with httpx.AsyncClient(timeout=REQUEST_TIMEOUT) as client:
        response = await client.post(
            f"{BASE_URL}/api/chat",
            json=body,
            headers={"Content-Type": "application/json"},
        )
        if response.status_code == 404:
            raise ValueError(
                f"Model '{_active_model}' not found in Ollama. "
                f"Install it with: ollama pull {_active_model}"
            )
        response.raise_for_status()
        return response.json()


async def fetch_ollama_models() -> list[dict]:
    """Fetch the list of locally installed models from Ollama."""
    async with httpx.AsyncClient(timeout=10) as client:
        response = await client.get(f"{BASE_URL}/api/tags")
        response.raise_for_status()
        data = response.json()
        return data.get("models", [])


# ============================================================
# MCP Server
# ============================================================

server = Server("cloto-mcp-ollama")


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="think",
            description=(
                "Generate a text response using a local Ollama model. "
                "No API key required — runs entirely on local hardware."
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
        Tool(
            name="list_models",
            description=(
                "List all locally installed Ollama models with size and modification date."
            ),
            inputSchema={
                "type": "object",
                "properties": {},
            },
        ),
        Tool(
            name="switch_model",
            description=(
                "Switch the active Ollama model for this session. "
                "The model must be locally installed (use list_models to check)."
            ),
            inputSchema={
                "type": "object",
                "properties": {
                    "model": {
                        "type": "string",
                        "description": "Model name to switch to (e.g., 'llama3.1', 'mistral', 'qwen2.5')",
                    },
                },
                "required": ["model"],
            },
        ),
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "think":
        return await handle_think(arguments)
    elif name == "list_models":
        return await handle_list_models()
    elif name == "switch_model":
        return await handle_switch_model(arguments)
    else:
        return [
            TextContent(
                type="text",
                text=json.dumps({"error": f"Unknown tool: {name}"}),
            )
        ]


async def handle_think(arguments: dict) -> list[TextContent]:
    """Handle 'think' tool: text generation via local Ollama."""
    try:
        agent = arguments.get("agent", {})
        message = arguments.get("message", {})
        context = arguments.get("context", [])

        messages = build_chat_messages(agent, message, context)
        response_data = await call_ollama_api(messages)
        content = parse_chat_content(response_data)

        return [
            TextContent(
                type="text", text=json.dumps({"type": "final", "content": content})
            )
        ]
    except httpx.ConnectError:
        return [
            TextContent(
                type="text",
                text=json.dumps({
                    "error": f"Cannot connect to Ollama at {BASE_URL}. "
                             f"Is Ollama running? Start it with: ollama serve"
                }),
            )
        ]
    except Exception as e:
        return [
            TextContent(
                type="text", text=json.dumps({"error": str(e)})
            )
        ]


async def handle_list_models() -> list[TextContent]:
    """Handle 'list_models' tool: list locally installed models."""
    try:
        models = await fetch_ollama_models()
        result = []
        for m in models:
            size_gb = m.get("size", 0) / (1024 ** 3)
            result.append({
                "name": m.get("name", ""),
                "size": f"{size_gb:.1f}GB",
                "modified_at": m.get("modified_at", ""),
                "family": m.get("details", {}).get("family", ""),
                "parameter_size": m.get("details", {}).get("parameter_size", ""),
                "quantization": m.get("details", {}).get("quantization_level", ""),
            })

        return [
            TextContent(
                type="text",
                text=json.dumps({
                    "active_model": _active_model,
                    "models": result,
                    "count": len(result),
                }),
            )
        ]
    except httpx.ConnectError:
        return [
            TextContent(
                type="text",
                text=json.dumps({
                    "error": f"Cannot connect to Ollama at {BASE_URL}. "
                             f"Is Ollama running? Start it with: ollama serve"
                }),
            )
        ]
    except Exception as e:
        return [
            TextContent(
                type="text", text=json.dumps({"error": str(e)})
            )
        ]


async def handle_switch_model(arguments: dict) -> list[TextContent]:
    """Handle 'switch_model' tool: change active model."""
    global _active_model

    model = arguments.get("model", "").strip()
    if not model:
        return [
            TextContent(
                type="text",
                text=json.dumps({"error": "Model name is required"}),
            )
        ]

    # Verify model is locally available
    try:
        models = await fetch_ollama_models()
        available_names = [m.get("name", "") for m in models]
        # Match both exact name and name without tag (e.g., "llama3.1" matches "llama3.1:latest")
        found = any(
            model == name or model == name.split(":")[0]
            for name in available_names
        )

        if not found:
            return [
                TextContent(
                    type="text",
                    text=json.dumps({
                        "error": f"Model '{model}' is not installed locally",
                        "available": available_names,
                        "hint": f"Install it with: ollama pull {model}",
                    }),
                )
            ]

        previous = _active_model
        _active_model = model

        return [
            TextContent(
                type="text",
                text=json.dumps({
                    "status": "switched",
                    "previous_model": previous,
                    "active_model": _active_model,
                }),
            )
        ]
    except httpx.ConnectError:
        return [
            TextContent(
                type="text",
                text=json.dumps({
                    "error": f"Cannot connect to Ollama at {BASE_URL}. "
                             f"Is Ollama running? Start it with: ollama serve"
                }),
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
