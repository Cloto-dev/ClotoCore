"""Cloto MCP STT Server — Local speech-to-text transcription.

Uses faster-whisper (CTranslate2) for efficient local transcription.
Supports GPU (CUDA) and CPU inference. Model is lazy-loaded on first use.

Tools:
  transcribe    — Transcribe an audio file to text
  list_models   — List available Whisper model sizes
"""

import asyncio
import os
import sys

sys.path.insert(0, os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")))

from common.mcp_utils import ToolRegistry, run_mcp_server

registry = ToolRegistry("cloto-mcp-stt")

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


@registry.tool(
    "transcribe",
    "Transcribe an audio file to text using Whisper. Supports WAV, MP3, FLAC, OGG, M4A.",
    {
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
)
async def handle_transcribe(args: dict) -> dict:
    file_path = args.get("file_path", "")
    if not file_path:
        return {"error": "file_path is required"}

    if not os.path.isfile(file_path):
        return {"error": f"File not found: {file_path}"}

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

        return {
            "text": text,
            "language": info.language,
            "language_probability": round(info.language_probability, 3),
            "duration": round(info.duration, 2),
            "segments": segments,
            "processing_time": elapsed,
        }
    except Exception as e:
        return {"error": str(e)}


@registry.tool(
    "list_models",
    "List available Whisper model sizes and current configuration.",
    {"type": "object", "properties": {}},
)
async def handle_list_models(args: dict) -> dict:
    return {
        "available": AVAILABLE_MODELS,
        "current": STT_MODEL,
        "device": STT_DEVICE,
        "language": STT_LANGUAGE,
        "loaded": _model is not None,
    }


if __name__ == "__main__":
    asyncio.run(run_mcp_server(registry))
