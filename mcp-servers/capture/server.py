"""Cloto MCP Vision Capture Server — Screenshot + image analysis.

Captures screenshots via mss (cross-platform) and analyzes images
using a hybrid approach: PaddleOCR (text extraction) + Ollama Vision (visual description).

Tools:
  capture_screen  — Take a screenshot and save as PNG
  analyze_image   — Send an image for OCR + vision analysis

OCR Modes (VISION_OCR_MODE env var):
  hybrid  — PaddleOCR + llava combined (default)
  vision  — llava only (original behavior)
  ocr     — PaddleOCR only
"""

import asyncio
import base64
import logging
import os
import sys
import uuid
from datetime import datetime, timezone

import httpx

sys.path.insert(0, os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")))

from common.mcp_utils import ToolRegistry, run_mcp_server

logger = logging.getLogger(__name__)

CAPTURE_OUTPUT_DIR = os.environ.get("CAPTURE_OUTPUT_DIR", "./data/captures")
VISION_OLLAMA_URL = os.environ.get("VISION_OLLAMA_URL", "http://localhost:11434")
VISION_MODEL = os.environ.get("VISION_MODEL", "llava")
VISION_OCR_MODE = os.environ.get("VISION_OCR_MODE", "hybrid")  # hybrid | vision | ocr

# ============================================================
# PaddleOCR (lazy init)
# ============================================================

_ocr_engine = None


def _get_ocr_engine():
    global _ocr_engine
    if _ocr_engine is not None:
        return _ocr_engine

    try:
        from paddleocr import PaddleOCR
        _ocr_engine = PaddleOCR(
            use_angle_cls=True,
            lang="japan",
            show_log=False,
        )
        logger.info("PaddleOCR initialized (lang=japan)")
    except ImportError:
        logger.warning("PaddleOCR not installed — OCR disabled. pip install paddleocr paddlepaddle")
        _ocr_engine = False  # Sentinel: tried and failed
    except Exception as e:
        logger.warning("PaddleOCR init failed: %s", e)
        _ocr_engine = False

    return _ocr_engine


async def _run_ocr(file_path: str) -> str | None:
    """Run PaddleOCR on an image file. Returns extracted text or None."""
    engine = _get_ocr_engine()
    if not engine:
        return None

    try:
        def _ocr():
            result = engine.ocr(file_path, cls=True)
            if not result or not result[0]:
                return ""
            lines = []
            for line in result[0]:
                text = line[1][0] if line[1] else ""
                confidence = line[1][1] if line[1] else 0
                if text and confidence > 0.5:
                    lines.append(text)
            return "\n".join(lines)

        return await asyncio.to_thread(_ocr)
    except Exception as e:
        logger.warning("OCR failed: %s", e)
        return None


# ============================================================
# Ollama Vision
# ============================================================


async def _run_vision(file_path: str, prompt: str, model: str) -> str | None:
    """Run Ollama vision model on an image. Returns description or None."""
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
            return result.get("response", "")
    except httpx.ConnectError:
        logger.warning("Cannot connect to Ollama at %s", VISION_OLLAMA_URL)
        return None
    except Exception as e:
        logger.warning("Vision analysis failed: %s", e)
        return None


# ============================================================
# MCP Server
# ============================================================

registry = ToolRegistry("cloto-mcp-capture")


@registry.tool(
    "capture_screen",
    "Take a screenshot of the primary monitor and save as PNG. Returns the file path.",
    {
        "type": "object",
        "properties": {
            "monitor": {
                "type": "integer",
                "description": "Monitor index (0=all, 1=primary, 2=secondary, etc.)",
            },
        },
    },
)
async def handle_capture_screen(arguments: dict) -> dict:
    monitor_idx = arguments.get("monitor", 1)

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

        size = await asyncio.to_thread(_capture)

        return {
            "status": "ok",
            "path": os.path.abspath(filepath),
            "width": size[0],
            "height": size[1],
            "monitor": monitor_idx,
        }
    except Exception as e:
        return {"error": str(e)}


@registry.tool(
    "analyze_image",
    "Analyze an image file using OCR (text extraction) and/or vision model (visual description). "
    f"Current mode: {VISION_OCR_MODE}. Returns combined analysis.",
    {
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
            "mode": {
                "type": "string",
                "description": "Analysis mode: hybrid, vision, ocr (overrides server default)",
            },
        },
        "required": ["file_path"],
    },
)
async def handle_analyze_image(arguments: dict) -> dict:
    file_path = arguments.get("file_path", "")
    if not file_path:
        return {"error": "file_path is required"}

    if not os.path.isfile(file_path):
        return {"error": f"File not found: {file_path}"}

    prompt = arguments.get("prompt", "Describe this image in detail.")
    model = arguments.get("model", VISION_MODEL)
    mode = arguments.get("mode", VISION_OCR_MODE)

    ocr_text = None
    vision_text = None

    try:
        # Run OCR and vision in parallel when in hybrid mode
        if mode == "hybrid":
            ocr_task = asyncio.create_task(_run_ocr(file_path))
            vision_task = asyncio.create_task(_run_vision(file_path, prompt, model))
            ocr_text, vision_text = await asyncio.gather(ocr_task, vision_task)
        elif mode == "ocr":
            ocr_text = await _run_ocr(file_path)
        elif mode == "vision":
            vision_text = await _run_vision(file_path, prompt, model)
        else:
            return {"error": f"Unknown mode: {mode}"}

        # Build combined response
        parts = []
        if ocr_text:
            parts.append(f"[OCR Text]\n{ocr_text}")
        if vision_text:
            parts.append(f"[Visual Description]\n{vision_text}")

        if not parts:
            parts.append("No analysis results available.")

        combined = "\n\n".join(parts)

        return {
            "status": "ok",
            "model": model,
            "mode": mode,
            "response": combined,
            "ocr_text": ocr_text or "",
            "vision_text": vision_text or "",
            "file": file_path,
        }
    except httpx.ConnectError:
        return {
            "error": f"Cannot connect to Ollama at {VISION_OLLAMA_URL}. Is Ollama running?",
            "hint": "Start Ollama with: ollama serve",
        }
    except Exception as e:
        return {"error": str(e)}


# ============================================================
# Ollama Auto-Start
# ============================================================

OLLAMA_BIN = os.environ.get("OLLAMA_BIN", "ollama")


async def _ensure_ollama_running():
    """Check if Ollama is reachable; if not, start it as a background process."""
    if VISION_OCR_MODE == "ocr":
        return  # No need for Ollama in OCR-only mode

    try:
        async with httpx.AsyncClient(timeout=3.0) as client:
            resp = await client.get(f"{VISION_OLLAMA_URL}/api/tags")
            if resp.status_code == 200:
                logger.info("Ollama already running at %s", VISION_OLLAMA_URL)
                return
    except (httpx.ConnectError, httpx.TimeoutException):
        pass

    # Try to find ollama binary
    import shutil
    import subprocess

    ollama_path = shutil.which(OLLAMA_BIN)
    if not ollama_path:
        # Check common Windows install paths
        candidates = [
            os.path.expandvars(r"%LOCALAPPDATA%\Programs\Ollama\ollama.exe"),
            r"C:\Program Files\Ollama\ollama.exe",
        ]
        for c in candidates:
            if os.path.isfile(c):
                ollama_path = c
                break

    if not ollama_path:
        logger.warning("Ollama not found — vision analysis will be unavailable")
        return

    logger.info("Starting Ollama: %s serve", ollama_path)
    subprocess.Popen(
        [ollama_path, "serve"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )

    # Wait for Ollama to become ready (up to 15 seconds)
    for i in range(30):
        await asyncio.sleep(0.5)
        try:
            async with httpx.AsyncClient(timeout=2.0) as client:
                resp = await client.get(f"{VISION_OLLAMA_URL}/api/tags")
                if resp.status_code == 200:
                    logger.info("Ollama started successfully (waited %.1fs)", (i + 1) * 0.5)
                    return
        except (httpx.ConnectError, httpx.TimeoutException):
            continue

    logger.warning("Ollama started but not responding after 15s — vision may fail")


# ============================================================
# Entry Point
# ============================================================


async def main():
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(name)s] %(levelname)s: %(message)s",
    )
    logger.info("Vision OCR mode: %s", VISION_OCR_MODE)
    await _ensure_ollama_running()
    await run_mcp_server(registry)


if __name__ == "__main__":
    asyncio.run(main())
