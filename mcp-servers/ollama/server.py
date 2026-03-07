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
import sys

import httpx

# Resolve parent directory for common module import.
_script_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.normpath(os.path.join(_script_dir, "..")))

from common.llm_provider import build_system_prompt, build_chat_messages
from common.mcp_utils import ToolRegistry, run_mcp_server
from common.validation import validate_dict, validate_list

# ============================================================
# Configuration (from environment variables)
# ============================================================

BASE_URL = os.environ.get("OLLAMA_BASE_URL", "http://localhost:11434")
MODEL_ID = os.environ.get("OLLAMA_MODEL", "glm-4.7-flash")
REQUEST_TIMEOUT = int(os.environ.get("OLLAMA_TIMEOUT_SECS", "120"))
MAX_PREDICT = int(os.environ.get("OLLAMA_MAX_PREDICT", "2048"))
ENABLE_THINKING = os.environ.get("OLLAMA_ENABLE_THINKING", "false").lower() == "true"

# Mutable session state (protected by _model_lock for concurrent access)
_active_model = MODEL_ID
_model_lock = asyncio.Lock()


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
    async with _model_lock:
        model = _active_model

    body: dict = {
        "model": model,
        "messages": messages,
        "stream": False,
        "options": {
            "num_predict": MAX_PREDICT,
            "repeat_penalty": 1.3,
            "repeat_last_n": 128,
        },
        "think": ENABLE_THINKING,
    }

    async with httpx.AsyncClient(timeout=REQUEST_TIMEOUT) as client:
        response = await client.post(
            f"{BASE_URL}/api/chat",
            json=body,
            headers={"Content-Type": "application/json"},
        )
        if response.status_code == 404:
            raise ValueError(
                f"Model '{model}' not found in Ollama. "
                f"Install it with: ollama pull {model}"
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

registry = ToolRegistry("cloto-mcp-ollama")


@registry.tool(
    "think",
    "Generate a text response using a local Ollama model. "
    "No API key required — runs entirely on local hardware.",
    {
        "type": "object",
        "properties": {
            "agent": {"type": "object", "description": "Agent metadata (name, description, metadata)"},
            "message": {"type": "object", "description": "User message with 'content' field"},
            "context": {"type": "array", "description": "Conversation context messages", "items": {"type": "object"}},
        },
        "required": ["agent", "message", "context"],
    },
)
async def handle_think(arguments: dict) -> dict:
    try:
        agent = validate_dict(arguments, "agent")
        message = validate_dict(arguments, "message")
        context = validate_list(arguments, "context")

        messages = build_chat_messages(agent, message, context)
        response_data = await call_ollama_api(messages)
        content = parse_chat_content(response_data)

        return {"type": "final", "content": content}
    except httpx.ConnectError:
        return {
            "error": f"Cannot connect to Ollama at {BASE_URL}. "
                     f"Is Ollama running? Start it with: ollama serve"
        }
    except Exception as e:
        return {"error": str(e)}


@registry.tool(
    "list_models",
    "List all locally installed Ollama models with size and modification date.",
    {"type": "object", "properties": {}},
)
async def handle_list_models(arguments: dict) -> dict:
    try:
        async with _model_lock:
            active = _active_model
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

        return {"active_model": active, "models": result, "count": len(result)}
    except httpx.ConnectError:
        return {
            "error": f"Cannot connect to Ollama at {BASE_URL}. "
                     f"Is Ollama running? Start it with: ollama serve"
        }
    except Exception as e:
        return {"error": str(e)}


@registry.tool(
    "switch_model",
    "Switch the active Ollama model for this session. "
    "The model must be locally installed (use list_models to check).",
    {
        "type": "object",
        "properties": {
            "model": {"type": "string", "description": "Model name to switch to (e.g., 'llama3.1', 'mistral', 'qwen2.5')"},
        },
        "required": ["model"],
    },
)
async def handle_switch_model(arguments: dict) -> dict:
    global _active_model

    model = arguments.get("model", "").strip()
    if not model:
        return {"error": "Model name is required"}

    try:
        models = await fetch_ollama_models()
        available_names = [m.get("name", "") for m in models]
        found = any(
            model == name or model == name.split(":")[0]
            for name in available_names
        )

        if not found:
            return {
                "error": f"Model '{model}' is not installed locally",
                "available": available_names,
                "hint": f"Install it with: ollama pull {model}",
            }

        async with _model_lock:
            previous = _active_model
            _active_model = model

        return {
            "status": "switched",
            "previous_model": previous,
            "active_model": model,
        }
    except httpx.ConnectError:
        return {
            "error": f"Cannot connect to Ollama at {BASE_URL}. "
                     f"Is Ollama running? Start it with: ollama serve"
        }
    except Exception as e:
        return {"error": str(e)}


if __name__ == "__main__":
    asyncio.run(run_mcp_server(registry))
