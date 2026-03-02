"""Cloto MCP Vision Capture Server — Screenshot + image analysis.

Captures screenshots via mss (cross-platform) and analyzes images
using Ollama's vision-capable models (e.g., llava, bakllava).

Tools:
  capture_screen  — Take a screenshot and save as PNG
  analyze_image   — Send an image to Ollama vision model for analysis
"""

import asyncio
import base64
import json
import os
import uuid
from datetime import datetime, timezone

import httpx
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

server = Server("cloto-mcp-capture")

CAPTURE_OUTPUT_DIR = os.environ.get("CAPTURE_OUTPUT_DIR", "./data/captures")
VISION_OLLAMA_URL = os.environ.get("VISION_OLLAMA_URL", "http://localhost:11434")
VISION_MODEL = os.environ.get("VISION_MODEL", "llava")


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="capture_screen",
            description="Take a screenshot of the primary monitor and save as PNG. Returns the file path.",
            inputSchema={
                "type": "object",
                "properties": {
                    "monitor": {
                        "type": "integer",
                        "description": "Monitor index (0=all, 1=primary, 2=secondary, etc.)",
                    },
                },
            },
        ),
        Tool(
            name="analyze_image",
            description="Analyze an image file using a local vision model (Ollama). Returns a text description.",
            inputSchema={
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the image file (PNG, JPEG, etc.)",
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Question or instruction for the vision model (default: 'Describe this image in detail.')",
                    },
                    "model": {
                        "type": "string",
                        "description": f"Ollama model to use (default: {VISION_MODEL})",
                    },
                },
                "required": ["file_path"],
            },
        ),
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "capture_screen":
        return await _handle_capture(arguments)
    elif name == "analyze_image":
        return await _handle_analyze(arguments)
    return [TextContent(type="text", text=json.dumps({"error": f"Unknown tool: {name}"}))]


async def _handle_capture(args: dict) -> list[TextContent]:
    monitor_idx = args.get("monitor", 1)

    try:
        import mss
        from PIL import Image

        os.makedirs(CAPTURE_OUTPUT_DIR, exist_ok=True)
        timestamp = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S")
        filename = f"capture_{timestamp}_{uuid.uuid4().hex[:6]}.png"
        filepath = os.path.join(CAPTURE_OUTPUT_DIR, filename)

        def _capture():
            with mss.mss() as sct:
                monitors = sct.monitors
                if monitor_idx >= len(monitors):
                    raise ValueError(
                        f"Monitor {monitor_idx} not found. Available: 0-{len(monitors) - 1}"
                    )
                screenshot = sct.grab(monitors[monitor_idx])
                img = Image.frombytes("RGB", screenshot.size, screenshot.bgra, "raw", "BGRX")
                img.save(filepath, "PNG")
                return screenshot.size

            return (0, 0)

        size = await asyncio.to_thread(_capture)

        return [
            TextContent(
                type="text",
                text=json.dumps(
                    {
                        "status": "ok",
                        "path": os.path.abspath(filepath),
                        "width": size[0],
                        "height": size[1],
                        "monitor": monitor_idx,
                    }
                ),
            )
        ]
    except Exception as e:
        return [TextContent(type="text", text=json.dumps({"error": str(e)}))]


async def _handle_analyze(args: dict) -> list[TextContent]:
    file_path = args.get("file_path", "")
    if not file_path:
        return [TextContent(type="text", text=json.dumps({"error": "file_path is required"}))]

    if not os.path.isfile(file_path):
        return [TextContent(type="text", text=json.dumps({"error": f"File not found: {file_path}"}))]

    prompt = args.get("prompt", "Describe this image in detail.")
    model = args.get("model", VISION_MODEL)

    try:
        with open(file_path, "rb") as f:
            image_data = base64.b64encode(f.read()).decode("utf-8")

        async with httpx.AsyncClient(timeout=120.0) as client:
            response = await client.post(
                f"{VISION_OLLAMA_URL}/api/generate",
                json={
                    "model": model,
                    "prompt": prompt,
                    "images": [image_data],
                    "stream": False,
                },
            )
            response.raise_for_status()
            result = response.json()

        return [
            TextContent(
                type="text",
                text=json.dumps(
                    {
                        "status": "ok",
                        "model": model,
                        "response": result.get("response", ""),
                        "file": file_path,
                    },
                    ensure_ascii=False,
                ),
            )
        ]
    except httpx.ConnectError:
        return [
            TextContent(
                type="text",
                text=json.dumps(
                    {
                        "error": f"Cannot connect to Ollama at {VISION_OLLAMA_URL}. Is Ollama running?",
                        "hint": "Start Ollama with: ollama serve",
                    }
                ),
            )
        ]
    except Exception as e:
        return [TextContent(type="text", text=json.dumps({"error": str(e)}))]


async def main():
    async with stdio_server() as (read_stream, write_stream):
        await server.run(
            read_stream, write_stream, server.create_initialization_options()
        )


if __name__ == "__main__":
    asyncio.run(main())
