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
import json
import os
import uuid
from datetime import datetime, timezone

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

server = Server("cloto-mcp-tts")

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


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="speak",
            description="Speak text aloud using the system TTS engine. Blocks until playback completes.",
            inputSchema={
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Text to speak aloud",
                    },
                },
                "required": ["text"],
            },
        ),
        Tool(
            name="synthesize",
            description="Save spoken text to a WAV file. Returns the file path.",
            inputSchema={
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
        ),
        Tool(
            name="list_voices",
            description="List available TTS voices on this system.",
            inputSchema={"type": "object", "properties": {}},
        ),
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "speak":
        return await _handle_speak(arguments)
    elif name == "synthesize":
        return await _handle_synthesize(arguments)
    elif name == "list_voices":
        return await _handle_list_voices()
    return [TextContent(type="text", text=json.dumps({"error": f"Unknown tool: {name}"}))]


async def _handle_speak(args: dict) -> list[TextContent]:
    text = args.get("text", "")
    if not text:
        return [TextContent(type="text", text=json.dumps({"error": "text is required"}))]

    try:
        engine = await asyncio.to_thread(_get_engine)

        def _speak():
            engine.say(text)
            engine.runAndWait()

        await asyncio.to_thread(_speak)
        return [
            TextContent(
                type="text",
                text=json.dumps(
                    {"status": "ok", "text": text, "chars": len(text)}
                ),
            )
        ]
    except Exception as e:
        return [TextContent(type="text", text=json.dumps({"error": str(e)}))]


async def _handle_synthesize(args: dict) -> list[TextContent]:
    text = args.get("text", "")
    if not text:
        return [TextContent(type="text", text=json.dumps({"error": "text is required"}))]

    filename = args.get("filename") or f"tts_{datetime.now(timezone.utc).strftime('%Y%m%d_%H%M%S')}_{uuid.uuid4().hex[:6]}.wav"

    try:
        os.makedirs(TTS_OUTPUT_DIR, exist_ok=True)
        filepath = os.path.join(TTS_OUTPUT_DIR, filename)

        engine = await asyncio.to_thread(_get_engine)

        def _save():
            engine.save_to_file(text, filepath)
            engine.runAndWait()

        await asyncio.to_thread(_save)

        return [
            TextContent(
                type="text",
                text=json.dumps(
                    {
                        "status": "ok",
                        "path": os.path.abspath(filepath),
                        "text": text,
                        "chars": len(text),
                    }
                ),
            )
        ]
    except Exception as e:
        return [TextContent(type="text", text=json.dumps({"error": str(e)}))]


async def _handle_list_voices() -> list[TextContent]:
    try:
        engine = await asyncio.to_thread(_get_engine)
        voices = engine.getProperty("voices")
        voice_list = [
            {"id": v.id, "name": v.name, "languages": getattr(v, "languages", [])}
            for v in voices
        ]
        current = engine.getProperty("voice")
        return [
            TextContent(
                type="text",
                text=json.dumps(
                    {"voices": voice_list, "current": current, "count": len(voice_list)}
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
