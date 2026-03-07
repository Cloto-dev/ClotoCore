"""
Shared search provider abstraction for Cloto MCP servers.
Fallback chain: SearXNG (self-hosted) → Tavily (cloud API) → DuckDuckGo (zero-config).
"""

import asyncio
import os
import sys
from abc import ABC, abstractmethod

import httpx

# ============================================================
# Configuration (read once at import time)
# ============================================================

PROVIDER = os.environ.get("CLOTO_SEARCH_PROVIDER", "auto")
SEARXNG_URL = os.environ.get("SEARXNG_URL", "http://localhost:8080")
TAVILY_API_KEY = os.environ.get("TAVILY_API_KEY", "")
REQUEST_TIMEOUT = int(os.environ.get("CLOTO_SEARCH_TIMEOUT", "15"))


# ============================================================
# Provider Abstraction
# ============================================================

class SearchProvider(ABC):
    name: str = "unknown"

    @abstractmethod
    async def search(self, query: str, max_results: int, language: str, time_range: str | None) -> list[dict]:
        ...


class SearXNGProvider(SearchProvider):
    """Self-hosted SearXNG — no API key, unlimited queries, full privacy."""
    name = "searxng"

    def __init__(self, base_url: str):
        self.base_url = base_url.rstrip("/")
        self.client = httpx.AsyncClient(timeout=REQUEST_TIMEOUT)

    async def aclose(self) -> None:
        await self.client.aclose()

    async def search(self, query: str, max_results: int, language: str, time_range: str | None) -> list[dict]:
        params: dict = {
            "q": query,
            "format": "json",
            "pageno": 1,
            "language": language,
        }
        if time_range:
            params["time_range"] = time_range

        resp = await self.client.get(f"{self.base_url}/search", params=params)
        resp.raise_for_status()
        data = resp.json()

        results = []
        for r in data.get("results", [])[:max_results]:
            results.append({
                "title": r.get("title", ""),
                "url": r.get("url", ""),
                "snippet": r.get("content", ""),
            })
        return results


class TavilyProvider(SearchProvider):
    """Tavily — AI-optimized search, 1000 free queries/month."""
    name = "tavily"

    def __init__(self, api_key: str):
        self.api_key = api_key
        self.client = httpx.AsyncClient(timeout=REQUEST_TIMEOUT)

    async def aclose(self) -> None:
        await self.client.aclose()

    async def search(self, query: str, max_results: int, language: str, time_range: str | None) -> list[dict]:
        payload: dict = {
            "query": query,
            "max_results": max_results,
            "api_key": self.api_key,
        }
        if time_range:
            day_map = {"day": 1, "week": 7, "month": 30, "year": 365}
            if time_range in day_map:
                payload["days"] = day_map[time_range]

        resp = await self.client.post("https://api.tavily.com/search", json=payload)
        resp.raise_for_status()
        data = resp.json()

        results = []
        for r in data.get("results", [])[:max_results]:
            results.append({
                "title": r.get("title", ""),
                "url": r.get("url", ""),
                "snippet": r.get("content", ""),
            })
        return results


class DuckDuckGoProvider(SearchProvider):
    """DuckDuckGo via ddgs — zero-config, no API key, rate-limited."""
    name = "duckduckgo"

    async def search(self, query: str, max_results: int, language: str, time_range: str | None) -> list[dict]:
        from ddgs import DDGS

        ddgs_timelimit = None
        if time_range:
            ddgs_timelimit = time_range[0]  # "d", "w", "m", "y"

        def _sync_search() -> list[dict]:
            with DDGS() as ddgs:
                raw = ddgs.text(query, max_results=max_results, timelimit=ddgs_timelimit)
                return [
                    {"title": r.get("title", ""), "url": r.get("href", ""), "snippet": r.get("body", "")}
                    for r in raw
                ]

        return await asyncio.to_thread(_sync_search)


class ChainProvider(SearchProvider):
    """Try providers in order, falling back on failure."""
    name = "chain"

    def __init__(self, providers: list[SearchProvider]):
        self.providers = providers

    async def aclose(self) -> None:
        for p in self.providers:
            if hasattr(p, "aclose"):
                await p.aclose()

    async def search(self, query: str, max_results: int, language: str, time_range: str | None) -> list[dict]:
        last_error: Exception | None = None
        for p in self.providers:
            try:
                return await p.search(query, max_results, language, time_range)
            except Exception as e:
                print(f"Provider {p.name} failed: {e}", file=sys.stderr)
                last_error = e
        raise last_error or RuntimeError("No search providers available")


def create_search_provider() -> SearchProvider:
    """Build provider (or chain) from CLOTO_SEARCH_PROVIDER env var.

    Supported values:
      "auto"    — SearXNG → Tavily (if key set) → DuckDuckGo
      "searxng" — SearXNG only
      "tavily"  — Tavily only
      "ddg"     — DuckDuckGo only
    """
    if PROVIDER == "auto":
        chain: list[SearchProvider] = [SearXNGProvider(SEARXNG_URL)]
        if TAVILY_API_KEY:
            chain.append(TavilyProvider(TAVILY_API_KEY))
        chain.append(DuckDuckGoProvider())
        return ChainProvider(chain)
    elif PROVIDER == "searxng":
        return SearXNGProvider(SEARXNG_URL)
    elif PROVIDER == "tavily":
        if not TAVILY_API_KEY:
            print("WARNING: TAVILY_API_KEY not set, search will fail", file=sys.stderr)
        return TavilyProvider(TAVILY_API_KEY)
    elif PROVIDER == "ddg":
        return DuckDuckGoProvider()
    else:
        raise ValueError(f"Unknown search provider: {PROVIDER}")
