"""Cloto MCP TTS Server — Local text-to-speech synthesis.

Uses pyttsx3 for cross-platform speech synthesis:
  - Windows: SAPI5
  - macOS: NSSpeechSynthesizer
  - Linux: espeak

Tools:
  speak         — Speak text aloud (blocking playback)
  synthesize    — Save speech to WAV file
  list_voices   — List available system voices
"""

import asyncio
import os
import sys
import uuid
from datetime import datetime, timezone

sys.path.insert(0, os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")))

from common.mcp_utils import ToolRegistry, run_mcp_server

registry = ToolRegistry("cloto-mcp-tts")

TTS_RATE = int(os.environ.get("TTS_RATE", "150"))
TTS_VOICE = os.environ.get("TTS_VOICE", "")
TTS_OUTPUT_DIR = os.environ.get("TTS_OUTPUT_DIR", "./data/speech")

_engine = None


def _get_engine():
    """Lazy-initialize pyttsx3 engine."""
    global _engine
    if _engine is None:
        import pyttsx3

        _engine = pyttsx3.init()
        _engine.setProperty("rate", TTS_RATE)
        if TTS_VOICE:
            _engine.setProperty("voice", TTS_VOICE)
    return _engine


@registry.tool(
    "speak",
    "Speak text aloud using the system TTS engine. Blocks until playback completes.",
    {
        "type": "object",
        "properties": {
            "text": {
                "type": "string",
                "description": "Text to speak aloud",
            },
        },
        "required": ["text"],
    },
)
async def handle_speak(args: dict) -> dict:
    text = args.get("text", "")
    if not text:
        return {"error": "text is required"}

    try:
        engine = await asyncio.to_thread(_get_engine)

        def _speak():
            engine.say(text)
            engine.runAndWait()

        await asyncio.to_thread(_speak)
        return {"status": "ok", "text": text, "chars": len(text)}
    except Exception as e:
        return {"error": str(e)}


@registry.tool(
    "synthesize",
    "Save spoken text to a WAV file. Returns the file path.",
    {
        "type": "object",
        "properties": {
            "text": {
                "type": "string",
                "description": "Text to synthesize",
            },
            "filename": {
                "type": "string",
                "description": "Output filename (optional, auto-generated if omitted)",
            },
        },
        "required": ["text"],
    },
)
async def handle_synthesize(args: dict) -> dict:
    text = args.get("text", "")
    if not text:
        return {"error": "text is required"}

    filename = args.get("filename") or f"tts_{datetime.now(timezone.utc).strftime('%Y%m%d_%H%M%S')}_{uuid.uuid4().hex[:6]}.wav"

    try:
        os.makedirs(TTS_OUTPUT_DIR, exist_ok=True)
        filepath = os.path.join(TTS_OUTPUT_DIR, filename)

        engine = await asyncio.to_thread(_get_engine)

        def _save():
            engine.save_to_file(text, filepath)
            engine.runAndWait()

        await asyncio.to_thread(_save)

        return {
            "status": "ok",
            "path": os.path.abspath(filepath),
            "text": text,
            "chars": len(text),
        }
    except Exception as e:
        return {"error": str(e)}


@registry.tool(
    "list_voices",
    "List available TTS voices on this system.",
    {"type": "object", "properties": {}},
)
async def handle_list_voices(args: dict) -> dict:
    try:
        engine = await asyncio.to_thread(_get_engine)
        voices = engine.getProperty("voices")
        voice_list = [
            {"id": v.id, "name": v.name, "languages": getattr(v, "languages", [])}
            for v in voices
        ]
        current = engine.getProperty("voice")
        return {"voices": voice_list, "current": current, "count": len(voice_list)}
    except Exception as e:
        return {"error": str(e)}


if __name__ == "__main__":
    asyncio.run(run_mcp_server(registry))
