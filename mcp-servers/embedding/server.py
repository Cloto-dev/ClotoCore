"""
Exiv MCP Server: Vector Embedding
Pluggable embedding provider with HTTP endpoint for inter-server communication.
Providers: api_openai (OpenAI-compatible API), onnx_miniml (local MiniLM ONNX).

Design: docs/KS22_MEMORY_DESIGN.md Section 5
"""

import asyncio
import json
import logging
import os
from abc import ABC, abstractmethod

import httpx
import numpy as np
from aiohttp import web
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

logger = logging.getLogger(__name__)

# ============================================================
# Configuration
# ============================================================

EMBEDDING_PROVIDER = os.environ.get("EMBEDDING_PROVIDER", "api_openai")
EMBEDDING_HTTP_PORT = int(os.environ.get("EMBEDDING_HTTP_PORT", "8401"))
EMBEDDING_API_KEY = os.environ.get("EMBEDDING_API_KEY", "")
EMBEDDING_API_URL = os.environ.get(
    "EMBEDDING_API_URL", "https://api.openai.com/v1/embeddings"
)
EMBEDDING_MODEL = os.environ.get("EMBEDDING_MODEL", "")  # provider-dependent default
EMBEDDING_TIMEOUT = int(os.environ.get("EMBEDDING_TIMEOUT_SECS", "30"))

# ONNX-specific
ONNX_MODEL_DIR = os.environ.get(
    "ONNX_MODEL_DIR", "data/models/all-MiniLM-L6-v2"
)

# ============================================================
# Provider Abstraction
# ============================================================


class EmbeddingProvider(ABC):
    """Abstract base class for embedding providers."""

    @abstractmethod
    async def initialize(self) -> None:
        """Initialize the provider (load model, create client, etc.)."""

    @abstractmethod
    async def embed(self, texts: list[str]) -> list[list[float]]:
        """Generate embeddings for a batch of texts."""

    @abstractmethod
    def dimensions(self) -> int:
        """Return the embedding dimensionality."""

    async def shutdown(self) -> None:
        """Clean up resources."""


# ============================================================
# api_openai Provider
# ============================================================


class OpenAIEmbeddingProvider(EmbeddingProvider):
    """OpenAI-compatible embedding API provider."""

    def __init__(self, api_key: str, api_url: str, model: str, timeout: int):
        self._api_key = api_key
        self._api_url = api_url
        self._model = model or "text-embedding-3-small"
        self._timeout = timeout
        self._client: httpx.AsyncClient | None = None
        self._dimensions = 1536  # text-embedding-3-small default

    async def initialize(self) -> None:
        if not self._api_key:
            raise ValueError(
                "EMBEDDING_API_KEY is required for api_openai provider"
            )
        self._client = httpx.AsyncClient(timeout=self._timeout)
        logger.info(
            "OpenAI embedding provider initialized (model=%s, url=%s)",
            self._model, self._api_url,
        )

    async def embed(self, texts: list[str]) -> list[list[float]]:
        if not self._client:
            raise RuntimeError("Provider not initialized")

        response = await self._client.post(
            self._api_url,
            headers={
                "Authorization": f"Bearer {self._api_key}",
                "Content-Type": "application/json",
            },
            json={"model": self._model, "input": texts},
        )
        response.raise_for_status()

        data = response.json()
        embeddings = [item["embedding"] for item in data["data"]]

        # Update dimensions from actual response
        if embeddings:
            self._dimensions = len(embeddings[0])

        # L2-normalize for consistent cosine similarity via dot product
        result = []
        for emb in embeddings:
            vec = np.array(emb, dtype=np.float32)
            norm = np.linalg.norm(vec)
            if norm > 1e-9:
                vec = vec / norm
            result.append(vec.tolist())

        return result

    def dimensions(self) -> int:
        return self._dimensions

    async def shutdown(self) -> None:
        if self._client:
            await self._client.aclose()
            self._client = None


# ============================================================
# onnx_miniml Provider
# ============================================================


class OnnxMiniLMProvider(EmbeddingProvider):
    """Local all-MiniLM-L6-v2 ONNX embedding provider."""

    def __init__(self, model_dir: str):
        self._model_dir = model_dir
        self._session = None
        self._tokenizer = None
        self._lock = asyncio.Lock()

    async def initialize(self) -> None:
        try:
            import onnxruntime as ort
            from tokenizers import Tokenizer
        except ImportError:
            raise ImportError(
                "onnx_miniml provider requires: pip install onnxruntime tokenizers\n"
                "Or: pip install exiv-mcp-embedding[onnx]"
            )

        model_path = os.path.join(self._model_dir, "model.onnx")
        tokenizer_path = os.path.join(self._model_dir, "tokenizer.json")

        if not os.path.exists(model_path):
            raise FileNotFoundError(
                f"ONNX model not found at {model_path}. "
                f"Download with: python mcp-servers/embedding/download_model.py"
            )

        if not os.path.exists(tokenizer_path):
            raise FileNotFoundError(
                f"Tokenizer not found at {tokenizer_path}. "
                f"Download with: python mcp-servers/embedding/download_model.py"
            )

        # Try DirectML (AMD GPU), fall back to CPU
        providers = []
        try:
            available = ort.get_available_providers()
            if "DmlExecutionProvider" in available:
                providers.append("DmlExecutionProvider")
                logger.info("Using DirectML (AMD GPU) for ONNX inference")
        except Exception:
            pass
        providers.append("CPUExecutionProvider")

        self._session = ort.InferenceSession(model_path, providers=providers)
        self._tokenizer = Tokenizer.from_file(tokenizer_path)
        self._tokenizer.enable_padding(pad_id=0, pad_token="[PAD]", length=128)
        self._tokenizer.enable_truncation(max_length=128)

        logger.info(
            "ONNX MiniLM provider initialized (dir=%s, providers=%s)",
            self._model_dir, providers,
        )

    async def embed(self, texts: list[str]) -> list[list[float]]:
        if not self._session or not self._tokenizer:
            raise RuntimeError("Provider not initialized")

        async with self._lock:
            return await asyncio.get_event_loop().run_in_executor(
                None, self._embed_sync, texts
            )

    def _embed_sync(self, texts: list[str]) -> list[list[float]]:
        """Synchronous embedding (run in executor to avoid blocking)."""
        encodings = self._tokenizer.encode_batch(texts)

        input_ids = np.array(
            [e.ids for e in encodings], dtype=np.int64
        )
        attention_mask = np.array(
            [e.attention_mask for e in encodings], dtype=np.int64
        )

        outputs = self._session.run(
            None,
            {"input_ids": input_ids, "attention_mask": attention_mask},
        )
        token_embeddings = outputs[0]  # (batch, seq_len, hidden_dim)

        # Mean pooling + L2 normalization
        mask_expanded = np.expand_dims(attention_mask, -1).astype(np.float32)
        sum_embeddings = np.sum(
            token_embeddings * mask_expanded, axis=1
        )
        sum_mask = np.clip(
            np.sum(mask_expanded, axis=1), a_min=1e-9, a_max=None
        )
        mean_pooled = sum_embeddings / sum_mask

        norms = np.linalg.norm(mean_pooled, axis=1, keepdims=True)
        norms = np.clip(norms, a_min=1e-9, a_max=None)
        normalized = mean_pooled / norms

        return normalized.tolist()

    def dimensions(self) -> int:
        return 384

    async def shutdown(self) -> None:
        self._session = None
        self._tokenizer = None


# ============================================================
# Provider Factory
# ============================================================


def create_provider() -> EmbeddingProvider:
    """Create an embedding provider based on configuration."""
    if EMBEDDING_PROVIDER == "api_openai":
        return OpenAIEmbeddingProvider(
            api_key=EMBEDDING_API_KEY,
            api_url=EMBEDDING_API_URL,
            model=EMBEDDING_MODEL,
            timeout=EMBEDDING_TIMEOUT,
        )
    elif EMBEDDING_PROVIDER == "onnx_miniml":
        return OnnxMiniLMProvider(model_dir=ONNX_MODEL_DIR)
    else:
        raise ValueError(
            f"Unknown embedding provider: {EMBEDDING_PROVIDER}. "
            f"Supported: api_openai, onnx_miniml"
        )


# ============================================================
# HTTP Endpoint (for KS22 inter-server communication)
# ============================================================

_provider: EmbeddingProvider | None = None


async def handle_embed(request: web.Request) -> web.Response:
    """POST /embed — Generate embeddings for input texts."""
    if _provider is None:
        return web.json_response(
            {"error": "Provider not initialized"}, status=503
        )

    try:
        body = await request.json()
    except Exception:
        return web.json_response(
            {"error": "Invalid JSON body"}, status=400
        )

    texts = body.get("texts")
    if not isinstance(texts, list) or not texts:
        return web.json_response(
            {"error": "'texts' must be a non-empty array of strings"},
            status=400,
        )

    # Limit batch size to prevent OOM
    if len(texts) > 100:
        return web.json_response(
            {"error": "Batch size exceeds limit (max 100)"}, status=400
        )

    try:
        embeddings = await _provider.embed(texts)
        return web.json_response({
            "embeddings": embeddings,
            "dimensions": _provider.dimensions(),
        })
    except Exception as e:
        logger.exception("Embedding failed")
        return web.json_response(
            {"error": f"Embedding failed: {e}"}, status=500
        )


async def run_http_server(port: int) -> None:
    """Run the HTTP embedding endpoint alongside MCP stdio."""
    app = web.Application()
    app.router.add_post("/embed", handle_embed)

    runner = web.AppRunner(app, access_log=None)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", port)
    await site.start()
    logger.info("HTTP embedding endpoint started on http://127.0.0.1:%d/embed", port)

    try:
        # Block until cancelled
        await asyncio.Event().wait()
    finally:
        await runner.cleanup()


# ============================================================
# MCP Server
# ============================================================

mcp_server = Server("exiv-mcp-embedding")


@mcp_server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="embed",
            description="Generate vector embeddings for input texts.",
            inputSchema={
                "type": "object",
                "properties": {
                    "texts": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Texts to embed (batch, max 100)",
                    }
                },
                "required": ["texts"],
            },
        ),
    ]


@mcp_server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name != "embed":
        return [
            TextContent(
                type="text",
                text=json.dumps({"error": f"Unknown tool: {name}"}),
            )
        ]

    if _provider is None:
        return [
            TextContent(
                type="text",
                text=json.dumps({"error": "Provider not initialized"}),
            )
        ]

    texts = arguments.get("texts", [])
    if not isinstance(texts, list) or not texts:
        return [
            TextContent(
                type="text",
                text=json.dumps(
                    {"error": "'texts' must be a non-empty array"}
                ),
            )
        ]

    if len(texts) > 100:
        return [
            TextContent(
                type="text",
                text=json.dumps(
                    {"error": "Batch size exceeds limit (max 100)"}
                ),
            )
        ]

    try:
        embeddings = await _provider.embed(texts)
        result = {
            "embeddings": embeddings,
            "dimensions": _provider.dimensions(),
        }
        return [TextContent(type="text", text=json.dumps(result))]
    except Exception as e:
        return [
            TextContent(
                type="text", text=json.dumps({"error": str(e)})
            )
        ]


# ============================================================
# Main
# ============================================================


async def main():
    global _provider

    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(name)s] %(levelname)s: %(message)s",
    )

    logger.info(
        "Starting embedding server (provider=%s, http_port=%d)",
        EMBEDDING_PROVIDER, EMBEDDING_HTTP_PORT,
    )

    _provider = create_provider()
    await _provider.initialize()

    # Start HTTP endpoint as background task
    http_task = asyncio.create_task(run_http_server(EMBEDDING_HTTP_PORT))

    try:
        async with stdio_server() as (read_stream, write_stream):
            await mcp_server.run(
                read_stream,
                write_stream,
                mcp_server.create_initialization_options(),
            )
    finally:
        http_task.cancel()
        try:
            await http_task
        except asyncio.CancelledError:
            pass
        await _provider.shutdown()
        logger.info("Embedding server shut down")


if __name__ == "__main__":
    asyncio.run(main())
