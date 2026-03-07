"""Tests for common.search module."""

import asyncio
import os
import sys
from unittest.mock import AsyncMock, patch, MagicMock

import pytest

sys.path.insert(0, os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")))

from common.search import (
    SearchProvider,
    SearXNGProvider,
    TavilyProvider,
    DuckDuckGoProvider,
    ChainProvider,
)


# ============================================================
# SearXNGProvider
# ============================================================


@pytest.mark.asyncio
async def test_searxng_provider_search():
    provider = SearXNGProvider("http://localhost:8080")
    mock_response = MagicMock()
    mock_response.json.return_value = {
        "results": [
            {"title": "Result 1", "url": "https://example.com/1", "content": "Snippet 1"},
            {"title": "Result 2", "url": "https://example.com/2", "content": "Snippet 2"},
        ]
    }
    mock_response.raise_for_status = MagicMock()

    with patch.object(provider.client, "get", new_callable=AsyncMock, return_value=mock_response):
        results = await provider.search("test query", 5, "en", None)

    assert len(results) == 2
    assert results[0]["title"] == "Result 1"
    assert results[0]["url"] == "https://example.com/1"
    assert results[0]["snippet"] == "Snippet 1"
    await provider.aclose()


# ============================================================
# TavilyProvider
# ============================================================


@pytest.mark.asyncio
async def test_tavily_provider_search():
    provider = TavilyProvider("test-api-key")
    mock_response = MagicMock()
    mock_response.json.return_value = {
        "results": [
            {"title": "Tavily Result", "url": "https://tavily.com/1", "content": "Tavily snippet"},
        ]
    }
    mock_response.raise_for_status = MagicMock()

    with patch.object(provider.client, "post", new_callable=AsyncMock, return_value=mock_response):
        results = await provider.search("test", 5, "en", "week")

    assert len(results) == 1
    assert results[0]["title"] == "Tavily Result"
    await provider.aclose()


# ============================================================
# ChainProvider
# ============================================================


@pytest.mark.asyncio
async def test_chain_provider_fallback():
    """ChainProvider should fall back to the next provider on failure."""
    failing = AsyncMock(spec=SearchProvider)
    failing.name = "failing"
    failing.search = AsyncMock(side_effect=RuntimeError("connection refused"))

    succeeding = AsyncMock(spec=SearchProvider)
    succeeding.name = "succeeding"
    succeeding.search = AsyncMock(return_value=[{"title": "OK", "url": "https://ok.com", "snippet": "works"}])

    chain = ChainProvider([failing, succeeding])
    results = await chain.search("test", 5, "en", None)

    assert len(results) == 1
    assert results[0]["title"] == "OK"
    failing.search.assert_called_once()
    succeeding.search.assert_called_once()


@pytest.mark.asyncio
async def test_chain_provider_all_fail():
    """ChainProvider should raise when all providers fail."""
    p1 = AsyncMock(spec=SearchProvider)
    p1.name = "p1"
    p1.search = AsyncMock(side_effect=RuntimeError("fail1"))

    p2 = AsyncMock(spec=SearchProvider)
    p2.name = "p2"
    p2.search = AsyncMock(side_effect=RuntimeError("fail2"))

    chain = ChainProvider([p1, p2])

    with pytest.raises(RuntimeError, match="fail2"):
        await chain.search("test", 5, "en", None)


# ============================================================
# create_search_provider factory
# ============================================================


def _reload_and_create():
    """Reload search module to pick up env changes, return (provider, module)."""
    import importlib
    import common.search as search_mod
    importlib.reload(search_mod)
    return search_mod.create_search_provider(), search_mod


def test_create_search_provider_auto():
    """Auto mode should create a ChainProvider."""
    with patch.dict(os.environ, {"CLOTO_SEARCH_PROVIDER": "auto"}, clear=False):
        provider, mod = _reload_and_create()
        assert isinstance(provider, mod.ChainProvider)


def test_create_search_provider_ddg():
    """DDG mode should create a DuckDuckGoProvider."""
    with patch.dict(os.environ, {"CLOTO_SEARCH_PROVIDER": "ddg"}, clear=False):
        provider, mod = _reload_and_create()
        assert isinstance(provider, mod.DuckDuckGoProvider)


def test_create_search_provider_searxng():
    """SearXNG mode should create a SearXNGProvider."""
    with patch.dict(os.environ, {"CLOTO_SEARCH_PROVIDER": "searxng"}, clear=False):
        provider, mod = _reload_and_create()
        assert isinstance(provider, mod.SearXNGProvider)


def test_create_search_provider_tavily():
    """Tavily mode should create a TavilyProvider."""
    with patch.dict(os.environ, {"CLOTO_SEARCH_PROVIDER": "tavily", "TAVILY_API_KEY": "key"}, clear=False):
        provider, mod = _reload_and_create()
        assert isinstance(provider, mod.TavilyProvider)
