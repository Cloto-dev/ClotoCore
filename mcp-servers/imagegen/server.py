"""Cloto MCP Image Generation Server — Stable Diffusion WebUI API.

Connects to a running Stable Diffusion WebUI (AUTOMATIC1111 / Forge)
instance via its REST API. Requires the WebUI to be started with --api flag.

Tools:
  generate_image  — Generate an image from a text prompt
  list_models     — List available SD models
"""

import asyncio
import base64
import json
import os
import sys
import uuid
from datetime import datetime, timezone

import httpx

sys.path.insert(0, os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")))

from common.mcp_utils import ToolRegistry, run_mcp_server

registry = ToolRegistry("cloto-mcp-imagegen")

SD_API_URL = os.environ.get("SD_API_URL", "http://127.0.0.1:7860")
SD_OUTPUT_DIR = os.environ.get("SD_OUTPUT_DIR", "./data/generated")


@registry.tool(
    "generate_image",
    "Generate an image from a text prompt using Stable Diffusion. Returns the saved file path.",
    {
        "type": "object",
        "properties": {
            "prompt": {
                "type": "string",
                "description": "Text prompt describing the desired image",
            },
            "negative_prompt": {
                "type": "string",
                "description": "Negative prompt (things to avoid)",
            },
            "steps": {
                "type": "integer",
                "description": "Sampling steps (default: 20)",
            },
            "width": {
                "type": "integer",
                "description": "Image width in pixels (default: 512)",
            },
            "height": {
                "type": "integer",
                "description": "Image height in pixels (default: 512)",
            },
            "cfg_scale": {
                "type": "number",
                "description": "CFG scale / guidance (default: 7.0)",
            },
            "seed": {
                "type": "integer",
                "description": "Random seed (-1 for random)",
            },
        },
        "required": ["prompt"],
    },
)
async def handle_generate(args: dict) -> dict:
    prompt = args.get("prompt", "")
    if not prompt:
        return {"error": "prompt is required"}

    payload = {
        "prompt": prompt,
        "negative_prompt": args.get("negative_prompt", ""),
        "steps": args.get("steps", 20),
        "width": args.get("width", 512),
        "height": args.get("height", 512),
        "cfg_scale": args.get("cfg_scale", 7.0),
        "seed": args.get("seed", -1),
    }

    try:
        async with httpx.AsyncClient(timeout=300.0) as client:
            response = await client.post(
                f"{SD_API_URL}/sdapi/v1/txt2img",
                json=payload,
            )
            response.raise_for_status()
            result = response.json()

        images = result.get("images", [])
        if not images:
            return {"error": "No images generated"}

        os.makedirs(SD_OUTPUT_DIR, exist_ok=True)
        timestamp = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S")
        filename = f"gen_{timestamp}_{uuid.uuid4().hex[:6]}.png"
        filepath = os.path.join(SD_OUTPUT_DIR, filename)

        image_data = base64.b64decode(images[0])
        with open(filepath, "wb") as f:
            f.write(image_data)

        info = result.get("info", "")
        seed_used = -1
        if isinstance(info, str):
            try:
                info_json = json.loads(info)
                seed_used = info_json.get("seed", -1)
            except (json.JSONDecodeError, TypeError):
                pass

        return {
            "status": "ok",
            "path": os.path.abspath(filepath),
            "prompt": prompt,
            "seed": seed_used,
            "width": payload["width"],
            "height": payload["height"],
            "steps": payload["steps"],
        }
    except httpx.ConnectError:
        return {
            "error": f"Cannot connect to Stable Diffusion WebUI at {SD_API_URL}",
            "hint": "Start SD WebUI with: webui.bat --api (or webui.sh --api)",
        }
    except Exception as e:
        return {"error": str(e)}


@registry.tool(
    "list_models",
    "List available Stable Diffusion models.",
    {"type": "object", "properties": {}},
)
async def handle_list_models(args: dict) -> dict:
    try:
        async with httpx.AsyncClient(timeout=30.0) as client:
            response = await client.get(f"{SD_API_URL}/sdapi/v1/sd-models")
            response.raise_for_status()
            models = response.json()

        model_list = [
            {"title": m.get("title", ""), "model_name": m.get("model_name", "")}
            for m in models
        ]

        return {"models": model_list, "count": len(model_list)}
    except httpx.ConnectError:
        return {
            "error": f"Cannot connect to Stable Diffusion WebUI at {SD_API_URL}",
            "hint": "Start SD WebUI with: webui.bat --api",
        }
    except Exception as e:
        return {"error": str(e)}


if __name__ == "__main__":
    asyncio.run(run_mcp_server(registry))
