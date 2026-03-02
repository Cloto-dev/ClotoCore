"""Cloto MCP STT Server — Local speech-to-text transcription.

Uses faster-whisper (CTranslate2) for efficient local transcription.
Supports GPU (CUDA) and CPU inference. Model is lazy-loaded on first use.

Tools:
  transcribe    — Transcribe an audio file to text
  list_models   — List available Whisper model sizes
"""

import asyncio
import json
import os

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

server = Server("cloto-mcp-stt")

STT_MODEL = os.environ.get("STT_MODEL", "base")
STT_DEVICE = os.environ.get("STT_DEVICE", "auto")
STT_LANGUAGE = os.environ.get("STT_LANGUAGE", "ja")

AVAILABLE_MODELS = ["tiny", "base", "small", "medium", "large-v3"]

_model = None


def _get_model():
    """Lazy-load the Whisper model on first transcription request."""
    global _model
    if _model is None:
        from faster_whisper import WhisperModel

        device = STT_DEVICE
        if device == "auto":
            try:
                import ctranslate2
                device = "cuda" if ctranslate2.get_cuda_device_count() > 0 else "cpu"
            except Exception:
                device = "cpu"

        compute_type = "float16" if device == "cuda" else "int8"
        _model = WhisperModel(STT_MODEL, device=device, compute_type=compute_type)
    return _model


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="transcribe",
            description="Transcribe an audio file to text using Whisper. Supports WAV, MP3, FLAC, OGG, M4A.",
            inputSchema={
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the audio file",
                    },
                    "language": {
                        "type": "string",
                        "description": f"Language code (default: {STT_LANGUAGE})",
                    },
                },
                "required": ["file_path"],
            },
        ),
        Tool(
            name="list_models",
            description="List available Whisper model sizes and current configuration.",
            inputSchema={"type": "object", "properties": {}},
        ),
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "transcribe":
        return await _handle_transcribe(arguments)
    elif name == "list_models":
        return await _handle_list_models()
    return [TextContent(type="text", text=json.dumps({"error": f"Unknown tool: {name}"}))]


async def _handle_transcribe(args: dict) -> list[TextContent]:
    file_path = args.get("file_path", "")
    if not file_path:
        return [TextContent(type="text", text=json.dumps({"error": "file_path is required"}))]

    if not os.path.isfile(file_path):
        return [TextContent(type="text", text=json.dumps({"error": f"File not found: {file_path}"}))]

    language = args.get("language", STT_LANGUAGE)

    try:
        import time

        start = time.monotonic()
        model = await asyncio.to_thread(_get_model)

        def _transcribe():
            segments_gen, info = model.transcribe(
                file_path,
                language=language,
                beam_size=5,
                vad_filter=True,
            )
            segments = []
            full_text_parts = []
            for seg in segments_gen:
                segments.append(
                    {
                        "start": round(seg.start, 2),
                        "end": round(seg.end, 2),
                        "text": seg.text.strip(),
                    }
                )
                full_text_parts.append(seg.text.strip())
            return " ".join(full_text_parts), segments, info

        text, segments, info = await asyncio.to_thread(_transcribe)
        elapsed = round(time.monotonic() - start, 2)

        return [
            TextContent(
                type="text",
                text=json.dumps(
                    {
                        "text": text,
                        "language": info.language,
                        "language_probability": round(info.language_probability, 3),
                        "duration": round(info.duration, 2),
                        "segments": segments,
                        "processing_time": elapsed,
                    },
                    ensure_ascii=False,
                ),
            )
        ]
    except Exception as e:
        return [TextContent(type="text", text=json.dumps({"error": str(e)}))]


async def _handle_list_models() -> list[TextContent]:
    return [
        TextContent(
            type="text",
            text=json.dumps(
                {
                    "available": AVAILABLE_MODELS,
                    "current": STT_MODEL,
                    "device": STT_DEVICE,
                    "language": STT_LANGUAGE,
                    "loaded": _model is not None,
                }
            ),
        )
    ]


async def main():
    async with stdio_server() as (read_stream, write_stream):
        await server.run(
            read_stream, write_stream, server.create_initialization_options()
        )


if __name__ == "__main__":
    asyncio.run(main())
