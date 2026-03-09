"""Benchmark tests for query embedding cache (Phase 1: baseline, Phase 2: cached).

Measures EmbeddingClient.embed() latency with a mock HTTP backend that simulates
realistic network delay. Identical queries should show cache hits after Phase 2.
"""

import asyncio
import time

import pytest

from cpersona.server import EmbeddingClient

# ---------------------------------------------------------------------------
# Mock embedding server (simulates 50ms network + compute latency)
# ---------------------------------------------------------------------------

MOCK_LATENCY_MS = 50
MOCK_DIMENSION = 384  # MiniLM-L6-v2 output dimension


class MockEmbeddingTransport:
    """Replaces httpx.AsyncClient to simulate embedding HTTP calls with controlled latency."""

    def __init__(self, latency_ms: int = MOCK_LATENCY_MS):
        self.latency_s = latency_ms / 1000.0
        self.call_count = 0

    async def post(self, url: str, json: dict = None, **kwargs):
        self.call_count += 1
        await asyncio.sleep(self.latency_s)
        texts = json.get("texts", []) if json else []
        # Deterministic fake embeddings (normalized, dimension-correct)
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
        self.status_code = 200

    def json(self):
        return self._data

    def raise_for_status(self):
        pass


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _create_client(mock: MockEmbeddingTransport) -> EmbeddingClient:
    """Create an EmbeddingClient with injected mock transport."""
    client = EmbeddingClient(
        mode="http",
        http_url="http://localhost:8401/embed",
    )
    client._client = mock
    return client


def _measure_ms(start: float) -> float:
    return (time.perf_counter() - start) * 1000


# ---------------------------------------------------------------------------
# Phase 1: Baseline benchmarks (no cache)
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_baseline_single_embed():
    """Single embed call should take ~MOCK_LATENCY_MS."""
    mock = MockEmbeddingTransport()
    client = _create_client(mock)

    start = time.perf_counter()
    result = await client.embed(["hello world"])
    elapsed = _measure_ms(start)

    assert result is not None
    assert len(result) == 1
    assert len(result[0]) == MOCK_DIMENSION
    assert mock.call_count == 1
    print(f"\n  Single embed: {elapsed:.1f}ms (expected ~{MOCK_LATENCY_MS}ms)")


@pytest.mark.asyncio
async def test_cached_repeated_identical_queries():
    """With cache, N identical queries should make only 1 HTTP call."""
    mock = MockEmbeddingTransport()
    client = _create_client(mock)
    query = "what is ClotoCore?"
    n_repeats = 5

    times = []
    for _ in range(n_repeats):
        start = time.perf_counter()
        await client.embed([query])
        times.append(_measure_ms(start))

    assert mock.call_count == 1, (
        f"Expected 1 HTTP call with cache, got {mock.call_count}"
    )
    assert client.cache_hits == n_repeats - 1
    assert client.cache_misses == 1

    total = sum(times)
    print(f"\n  {n_repeats}x identical query (cached):")
    print(f"    Total: {total:.1f}ms")
    print(f"    Per-call: {[f'{t:.1f}' for t in times]}")
    print(f"    HTTP calls: {mock.call_count}")
    print(f"    Cache hits: {client.cache_hits}, misses: {client.cache_misses}")


@pytest.mark.asyncio
async def test_baseline_distinct_queries():
    """Distinct queries should always make HTTP calls (baseline and cached)."""
    mock = MockEmbeddingTransport()
    client = _create_client(mock)
    queries = [f"query_{i}" for i in range(5)]

    for q in queries:
        await client.embed([q])

    assert mock.call_count == len(queries)
    print(f"\n  {len(queries)} distinct queries: {mock.call_count} HTTP calls")


@pytest.mark.asyncio
async def test_cached_embed_deterministic():
    """Same text should produce identical embeddings; 2nd call from cache."""
    mock = MockEmbeddingTransport()
    client = _create_client(mock)

    r1 = await client.embed(["test text"])
    r2 = await client.embed(["test text"])

    assert r1 == r2
    assert mock.call_count == 1  # Cache: only 1st call hits the server
    assert client.cache_hits == 1


@pytest.mark.asyncio
async def test_baseline_batch_vs_single():
    """Batch embed of N texts = 1 HTTP call; N single calls = N HTTP calls."""
    mock_batch = MockEmbeddingTransport()
    client_batch = _create_client(mock_batch)

    mock_single = MockEmbeddingTransport()
    client_single = _create_client(mock_single)

    texts = [f"text_{i}" for i in range(5)]

    # Batch
    start = time.perf_counter()
    batch_result = await client_batch.embed(texts)
    batch_time = _measure_ms(start)

    # Single
    start = time.perf_counter()
    single_results = []
    for t in texts:
        r = await client_single.embed([t])
        single_results.append(r[0] if r else None)
    single_time = _measure_ms(start)

    assert mock_batch.call_count == 1, "Batch should be 1 HTTP call"
    assert mock_single.call_count == 5, "Single calls should be 5 HTTP calls"
    print(f"\n  Batch (1 call): {batch_time:.1f}ms")
    print(f"  Single (5 calls): {single_time:.1f}ms")
    print(f"  Speedup: {single_time / batch_time:.1f}x")


# ---------------------------------------------------------------------------
# Phase 2: Cache behavior tests
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_cache_ttl_expiry():
    """Expired entries should be evicted and re-fetched."""
    mock = MockEmbeddingTransport(latency_ms=10)
    client = EmbeddingClient(mode="http", http_url="http://localhost/embed",
                             cache_size=256, cache_ttl=0)  # TTL=0 → immediate expiry
    client._client = mock

    await client.embed(["ttl test"])
    await client.embed(["ttl test"])  # Should miss (expired)

    assert mock.call_count == 2
    assert client.cache_hits == 0
    assert client.cache_misses == 2


@pytest.mark.asyncio
async def test_cache_lru_eviction():
    """When cache is full, oldest entries should be evicted."""
    mock = MockEmbeddingTransport(latency_ms=1)
    client = EmbeddingClient(mode="http", http_url="http://localhost/embed",
                             cache_size=3, cache_ttl=300)
    client._client = mock

    # Fill cache with 3 entries: [a, b, c]
    await client.embed(["a"])
    await client.embed(["b"])
    await client.embed(["c"])
    assert mock.call_count == 3

    # Access "a" → moves to end: [b, c, a]
    await client.embed(["a"])
    assert mock.call_count == 3  # Cache hit
    assert client.cache_hits == 1

    # Add "d" → evicts "b" (LRU): [c, a, d]
    await client.embed(["d"])
    assert mock.call_count == 4

    # "b" evicted → miss, re-fetch → evicts "c" (LRU): [a, d, b]
    await client.embed(["b"])
    assert mock.call_count == 5

    # "a" should still be cached (was accessed recently)
    await client.embed(["a"])
    assert mock.call_count == 5  # Hit

    # "d" should still be cached
    await client.embed(["d"])
    assert mock.call_count == 5  # Hit


@pytest.mark.asyncio
async def test_cache_batch_bypass():
    """Batch calls (len > 1) should bypass cache entirely."""
    mock = MockEmbeddingTransport(latency_ms=1)
    client = _create_client(mock)

    # Batch call
    await client.embed(["x", "y"])
    assert mock.call_count == 1
    assert client.cache_hits == 0
    assert client.cache_misses == 0  # Batch doesn't touch cache

    # Single call for same text should still miss (not cached from batch)
    await client.embed(["x"])
    assert mock.call_count == 2
    assert client.cache_misses == 1


@pytest.mark.asyncio
async def test_cache_stats_accuracy():
    """Cache hit/miss counters should be accurate."""
    mock = MockEmbeddingTransport(latency_ms=1)
    client = _create_client(mock)

    await client.embed(["q1"])  # miss
    await client.embed(["q2"])  # miss
    await client.embed(["q1"])  # hit
    await client.embed(["q1"])  # hit
    await client.embed(["q3"])  # miss
    await client.embed(["q2"])  # hit

    assert client.cache_misses == 3
    assert client.cache_hits == 3
    assert mock.call_count == 3
