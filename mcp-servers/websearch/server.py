"""
Cloto MCP Server: Web Search
Multi-provider web search with page content extraction.
Supports SearXNG (self-hosted) and Tavily (cloud API).
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

PROVIDER = os.environ.get("CLOTO_SEARCH_PROVIDER", "tavily")  # searxng | tavily
SEARXNG_URL = os.environ.get("SEARXNG_URL", "http://localhost:8080")
TAVILY_API_KEY = os.environ.get("TAVILY_API_KEY", "")
DEFAULT_MAX_RESULTS = 5
FETCH_MAX_LENGTH = 10000
REQUEST_TIMEOUT = 15


# ============================================================
# Provider Abstraction
# ============================================================

class SearchProvider(ABC):
    @abstractmethod
    async def search(self, query: str, max_results: int, language: str, time_range: str | None) -> list[dict]:
        ...


class SearXNGProvider(SearchProvider):
    """Self-hosted SearXNG — no API key, unlimited queries, full privacy."""

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


def create_provider() -> SearchProvider:
    if PROVIDER == "searxng":
        return SearXNGProvider(SEARXNG_URL)
    elif PROVIDER == "tavily":
        if not TAVILY_API_KEY:
            print("WARNING: TAVILY_API_KEY not set, search will fail", file=sys.stderr)
        return TavilyProvider(TAVILY_API_KEY)
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
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "web_search":
        return await handle_web_search(arguments)
    elif name == "fetch_page":
        return await handle_fetch_page(arguments)
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
            "error": f"Search failed ({PROVIDER}): {e}",
            "provider": PROVIDER,
            "query": query,
        }))]


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
