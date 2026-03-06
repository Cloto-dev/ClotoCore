"""
Cloto MCP Server: Web Search
Multi-provider web search with page content extraction.
Fallback chain: SearXNG (self-hosted) → Tavily (cloud API) → DuckDuckGo (zero-config).
"""

import asyncio
import json
import os
import sys
from abc import ABC, abstractmethod

import httpx
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

# ============================================================
# Configuration
# ============================================================

PROVIDER = os.environ.get("CLOTO_SEARCH_PROVIDER", "auto")
SEARXNG_URL = os.environ.get("SEARXNG_URL", "http://localhost:8080")
TAVILY_API_KEY = os.environ.get("TAVILY_API_KEY", "")
DEFAULT_MAX_RESULTS = 5
FETCH_MAX_LENGTH = 10000
REQUEST_TIMEOUT = 15


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

    async def search(self, query: str, max_results: int, language: str, time_range: str | None) -> list[dict]:
        last_error: Exception | None = None
        for p in self.providers:
            try:
                return await p.search(query, max_results, language, time_range)
            except Exception as e:
                print(f"Provider {p.name} failed: {e}", file=sys.stderr)
                last_error = e
        raise last_error or RuntimeError("No search providers available")


def create_provider() -> SearchProvider:
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


provider = create_provider()


# ============================================================
# Page Fetcher
# ============================================================

async def fetch_page_content(url: str, max_length: int) -> str:
    """Fetch a URL and extract text content."""
    client = httpx.AsyncClient(timeout=REQUEST_TIMEOUT, follow_redirects=True)
    try:
        resp = await client.get(url, headers={
            "User-Agent": "ClotoCore/0.4 (Web Search MCP Server)",
            "Accept": "text/html,application/xhtml+xml,text/plain",
        })
        resp.raise_for_status()
        content_type = resp.headers.get("content-type", "")

        if "text/html" in content_type:
            return html_to_text(resp.text)[:max_length]
        elif "text/plain" in content_type or "application/json" in content_type:
            return resp.text[:max_length]
        else:
            return f"[Unsupported content type: {content_type}]"
    except Exception as e:
        return f"[Error fetching {url}: {e}]"
    finally:
        await client.aclose()


def html_to_text(html: str) -> str:
    """Simple HTML to text conversion without heavy dependencies."""
    import re
    # Remove script and style blocks
    text = re.sub(r'<script[^>]*>.*?</script>', '', html, flags=re.DOTALL | re.IGNORECASE)
    text = re.sub(r'<style[^>]*>.*?</style>', '', text, flags=re.DOTALL | re.IGNORECASE)
    # Convert common block elements to newlines
    text = re.sub(r'<(?:p|div|h[1-6]|li|br|tr)[^>]*>', '\n', text, flags=re.IGNORECASE)
    # Remove remaining tags
    text = re.sub(r'<[^>]+>', '', text)
    # Decode common entities
    text = text.replace('&amp;', '&').replace('&lt;', '<').replace('&gt;', '>')
    text = text.replace('&quot;', '"').replace('&#39;', "'").replace('&nbsp;', ' ')
    # Collapse whitespace
    text = re.sub(r'\n\s*\n', '\n\n', text)
    text = re.sub(r' +', ' ', text)
    return text.strip()


# ============================================================
# Provider Health Check
# ============================================================

async def check_provider_status(name: str) -> dict:
    """Check if a specific provider is configured and reachable."""
    if name == "searxng":
        configured = bool(SEARXNG_URL)
        if not configured:
            return {"name": name, "configured": False, "reachable": False}
        try:
            async with httpx.AsyncClient(timeout=5) as client:
                resp = await client.get(f"{SEARXNG_URL}/")
                reachable = resp.status_code < 500
        except Exception:
            reachable = False
        return {
            "name": name,
            "configured": True,
            "reachable": reachable,
            "url": SEARXNG_URL,
            "setup_hint": "Run 'docker compose up -d' in the ClotoCore project root." if not reachable else None,
        }
    elif name == "tavily":
        configured = bool(TAVILY_API_KEY)
        return {
            "name": name,
            "configured": configured,
            "reachable": configured,  # If key is set, API is reachable (cloud service)
            "setup_hint": "Register at https://tavily.com (free, no credit card) and add TAVILY_API_KEY to .env." if not configured else None,
        }
    elif name == "duckduckgo":
        return {
            "name": name,
            "configured": True,
            "reachable": True,  # Best-effort, always "available"
            "note": "Zero-config fallback. Rate-limited and unstable — upgrade to SearXNG or Tavily recommended.",
        }
    return {"name": name, "configured": False, "reachable": False}


# ============================================================
# MCP Server
# ============================================================

server = Server("cloto-mcp-websearch")


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="web_search",
            description=(
                "Search the web and return relevant results with titles, URLs, "
                "and snippets. Use this to find current information, documentation, "
                "news, or any web-based knowledge."
            ),
            inputSchema={
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query",
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results to return (default: 5, max: 20)",
                    },
                    "language": {
                        "type": "string",
                        "description": "Language code (e.g., 'en', 'ja'). Default: 'en'",
                    },
                    "time_range": {
                        "type": "string",
                        "enum": ["day", "week", "month", "year"],
                        "description": "Filter results by recency",
                    },
                },
                "required": ["query"],
            },
        ),
        Tool(
            name="fetch_page",
            description=(
                "Fetch a web page and extract its text content. "
                "Use after web_search to read the full content of a result."
            ),
            inputSchema={
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch",
                    },
                    "max_length": {
                        "type": "integer",
                        "description": "Maximum characters to return (default: 10000)",
                    },
                },
                "required": ["url"],
            },
        ),
        Tool(
            name="search_status",
            description=(
                "Check which web search providers are configured and reachable. "
                "Returns the status of each provider in the fallback chain "
                "(SearXNG, Tavily, DuckDuckGo) with setup hints for unconfigured providers. "
                "Use this when search fails or when the user asks about search capabilities."
            ),
            inputSchema={
                "type": "object",
                "properties": {},
            },
        ),
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "web_search":
        return await handle_web_search(arguments)
    elif name == "fetch_page":
        return await handle_fetch_page(arguments)
    elif name == "search_status":
        return await handle_search_status()
    else:
        return [TextContent(type="text", text=json.dumps({"error": f"Unknown tool: {name}"}))]


async def handle_web_search(arguments: dict) -> list[TextContent]:
    query = arguments.get("query", "")
    max_results = min(arguments.get("max_results", DEFAULT_MAX_RESULTS), 20)
    language = arguments.get("language", "en")
    time_range = arguments.get("time_range")

    if not query.strip():
        return [TextContent(type="text", text=json.dumps({"error": "Empty query"}))]

    try:
        results = await provider.search(query, max_results, language, time_range)
        response = {
            "provider": PROVIDER,
            "query": query,
            "results": results,
            "total_results": len(results),
        }
        return [TextContent(type="text", text=json.dumps(response, ensure_ascii=False))]
    except Exception as e:
        return [TextContent(type="text", text=json.dumps({
            "error": f"All search providers failed. Last error: {e}",
            "provider": PROVIDER,
            "query": query,
        }))]


async def handle_search_status() -> list[TextContent]:
    statuses = await asyncio.gather(
        check_provider_status("searxng"),
        check_provider_status("tavily"),
        check_provider_status("duckduckgo"),
    )
    chain = list(statuses)

    # Determine which provider will actually handle searches
    active = "none"
    for s in chain:
        if s["reachable"]:
            active = s["name"]
            break

    response = {
        "mode": PROVIDER,
        "active_provider": active,
        "chain": chain,
    }
    return [TextContent(type="text", text=json.dumps(response, ensure_ascii=False))]


async def handle_fetch_page(arguments: dict) -> list[TextContent]:
    url = arguments.get("url", "")
    max_length = arguments.get("max_length", FETCH_MAX_LENGTH)

    if not url.strip():
        return [TextContent(type="text", text=json.dumps({"error": "Empty URL"}))]

    content = await fetch_page_content(url, max_length)
    response = {
        "url": url,
        "content": content,
        "length": len(content),
        "truncated": len(content) >= max_length,
    }
    return [TextContent(type="text", text=json.dumps(response, ensure_ascii=False))]


# ============================================================
# Entry Point
# ============================================================

async def main():
    async with stdio_server() as (read_stream, write_stream):
        await server.run(read_stream, write_stream, server.create_initialization_options())


if __name__ == "__main__":
    asyncio.run(main())
