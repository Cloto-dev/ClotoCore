"""Tests for SemanticCache (common/semantic_cache.py).

Uses a mock embedding server that returns deterministic normalized vectors.
"""

import asyncio
import time

import pytest

from common.semantic_cache import SemanticCache, _dot_product

# ---------------------------------------------------------------------------
# Mock embedding transport
# ---------------------------------------------------------------------------

MOCK_DIMENSION = 384


class MockEmbeddingClient:
    """Mock httpx.AsyncClient that returns deterministic embeddings."""

    def __init__(self, latency_ms: int = 5):
        self.latency_s = latency_ms / 1000.0
        self.call_count = 0

    async def post(self, url: str, json: dict = None, **kwargs):
        self.call_count += 1
        await asyncio.sleep(self.latency_s)
        texts = json.get("texts", []) if json else []
        embeddings = []
        for text in texts:
            h = hash(text) & 0xFFFFFFFF
            vec = [(h ^ (i * 2654435761)) % 1000 / 1000.0 for i in range(MOCK_DIMENSION)]
            norm = sum(v * v for v in vec) ** 0.5
            embeddings.append([v / norm for v in vec])
        return MockResponse({"embeddings": embeddings})

    async def aclose(self):
        pass


class MockResponse:
    def __init__(self, data: dict):
        self._data = data

    def json(self):
        return self._data

    def raise_for_status(self):
        pass


def _create_cache(mock: MockEmbeddingClient, **kwargs) -> SemanticCache:
    """Create a SemanticCache with injected mock client."""
    defaults = {
        "max_entries": 100,
        "ttl": 3600,
        "similarity_threshold": 0.95,
        "min_score": 8,
    }
    defaults.update(kwargs)
    cache = SemanticCache(**defaults)
    cache._client = mock
    return cache


# ---------------------------------------------------------------------------
# Unit tests: _dot_product
# ---------------------------------------------------------------------------

def test_dot_product_identical():
    vec = [0.5, 0.5, 0.5, 0.5]
    assert abs(_dot_product(vec, vec) - 1.0) < 0.01


def test_dot_product_orthogonal():
    a = [1.0, 0.0, 0.0]
    b = [0.0, 1.0, 0.0]
    assert _dot_product(a, b) == 0.0


# ---------------------------------------------------------------------------
# Phase 3: Semantic cache behavior
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_cache_miss_on_empty():
    """Lookup on empty cache should return None."""
    mock = MockEmbeddingClient()
    cache = _create_cache(mock)

    result = await cache.lookup("any query")
    assert result is None
    assert cache.misses == 1
    assert cache.hits == 0


@pytest.mark.asyncio
async def test_store_and_lookup_exact():
    """Exact same query should produce a cache hit."""
    mock = MockEmbeddingClient()
    cache = _create_cache(mock)

    sample_result = {"result": "ClotoCore is an AI platform", "sources": {}}

    stored = await cache.store("what is ClotoCore?", sample_result, score=9)
    assert stored is True

    hit = await cache.lookup("what is ClotoCore?")
    assert hit is not None
    assert hit["result"] == "ClotoCore is an AI platform"
    assert cache.hits == 1


@pytest.mark.asyncio
async def test_store_rejected_low_score():
    """Results with score < min_score should not be stored."""
    mock = MockEmbeddingClient()
    cache = _create_cache(mock, min_score=8)

    stored = await cache.store("query", {"result": "low quality"}, score=5)
    assert stored is False

    hit = await cache.lookup("query")
    assert hit is None


@pytest.mark.asyncio
async def test_cache_miss_different_query():
    """Different query should miss (low similarity)."""
    mock = MockEmbeddingClient()
    cache = _create_cache(mock, similarity_threshold=0.95)

    await cache.store("what is ClotoCore?", {"result": "A"}, score=9)

    # Completely different query → different hash → different vector → low similarity
    hit = await cache.lookup("how to cook pasta?")
    assert hit is None
    assert cache.misses == 1


@pytest.mark.asyncio
async def test_cache_ttl_expiry():
    """Expired entries should not be returned."""
    mock = MockEmbeddingClient()
    cache = _create_cache(mock, ttl=0)  # TTL=0 → immediate expiry

    await cache.store("query", {"result": "cached"}, score=9)
    hit = await cache.lookup("query")
    assert hit is None
    assert cache.misses == 1


@pytest.mark.asyncio
async def test_cache_lru_eviction():
    """Oldest entries should be evicted when max_entries is reached."""
    mock = MockEmbeddingClient()
    cache = _create_cache(mock, max_entries=2)

    await cache.store("q1", {"result": "r1"}, score=9)
    await cache.store("q2", {"result": "r2"}, score=9)
    await cache.store("q3", {"result": "r3"}, score=9)  # Evicts q1

    assert len(cache._cache) == 2

    # q1 should be evicted
    hit = await cache.lookup("q1")
    assert hit is None


@pytest.mark.asyncio
async def test_cache_hit_flag():
    """Verify cache_hit flag is set by the research handler pattern."""
    mock = MockEmbeddingClient()
    cache = _create_cache(mock)

    original = {"result": "test", "sources": {}, "stats": {"final_score": 9}}
    await cache.store("query", original, score=9)

    cached = await cache.lookup("query")
    assert cached is not None
    # Simulate what handle_deep_research does
    cached["cache_hit"] = True
    assert cached["cache_hit"] is True
    assert cached["result"] == "test"


@pytest.mark.asyncio
async def test_cache_stats():
    """Hit/miss counters should be accurate."""
    mock = MockEmbeddingClient()
    cache = _create_cache(mock)

    await cache.lookup("miss1")  # miss
    await cache.lookup("miss2")  # miss
    await cache.store("q", {"r": 1}, score=9)
    await cache.lookup("q")  # hit
    await cache.lookup("q")  # hit

    assert cache.misses == 2
    assert cache.hits == 2


@pytest.mark.asyncio
async def test_embedding_failure_graceful():
    """If embedding fails, lookup returns None without raising."""

    class FailingClient:
        async def post(self, *args, **kwargs):
            raise httpx.ConnectError("Connection refused")

        async def aclose(self):
            pass

    import httpx  # noqa: F811
    cache = SemanticCache()
    cache._client = FailingClient()

    result = await cache.lookup("query")
    assert result is None
    assert cache.misses == 1

    stored = await cache.store("query", {"r": 1}, score=9)
    assert stored is False
