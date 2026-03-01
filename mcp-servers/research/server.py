"""
Cloto MCP Server: Deep Research
Agentic RAG with self-evaluation loop, query expansion, and multi-hop reasoning.
Ported from ai_karin's ResearchCoordinator + RagEngine architecture.

Architecture:
  Query → Expand → Search → Extract → Evaluate → (Score < threshold? Refine & loop) → Synthesize
  Uses LLM Proxy (8082) with X-LLM-Provider header for model selection.
"""

import asyncio
import json
import os
import re
import sys

sys.path.insert(0, os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")))

import httpx
from mcp.server import Server
from mcp.types import TextContent, Tool

# ============================================================
# Configuration
# ============================================================

LLM_PROXY_URL = os.environ.get("LLM_PROXY_URL", "http://127.0.0.1:8082/v1/chat/completions")
SEARCH_PROVIDER = os.environ.get("CLOTO_SEARCH_PROVIDER", "tavily")
SEARXNG_URL = os.environ.get("SEARXNG_URL", "http://localhost:8080")
TAVILY_API_KEY = os.environ.get("TAVILY_API_KEY", "")

# Model roles — configurable via env
EXTRACT_PROVIDER = os.environ.get("RESEARCH_EXTRACT_PROVIDER", "cerebras")  # Fast, bulk text
EVALUATE_PROVIDER = os.environ.get("RESEARCH_EVALUATE_PROVIDER", "deepseek")  # Reasoning
SYNTHESIZE_PROVIDER = os.environ.get("RESEARCH_SYNTHESIZE_PROVIDER", "deepseek")  # High quality

MAX_RETRIES = int(os.environ.get("RESEARCH_MAX_RETRIES", "3"))
PASS_SCORE = int(os.environ.get("RESEARCH_PASS_SCORE", "6"))
CACHE_SCORE = 8
REQUEST_TIMEOUT = 30
SEARCH_TIMEOUT = 15


# ============================================================
# LLM Proxy Client
# ============================================================

async def call_llm(provider: str, prompt: str, system: str | None = None) -> str:
    """Call LLM via kernel proxy with provider selection."""
    messages = []
    if system:
        messages.append({"role": "system", "content": system})
    messages.append({"role": "user", "content": prompt})

    async with httpx.AsyncClient(timeout=REQUEST_TIMEOUT) as client:
        resp = await client.post(
            LLM_PROXY_URL,
            headers={"X-LLM-Provider": provider, "Content-Type": "application/json"},
            json={"messages": messages},
        )
        resp.raise_for_status()
        data = resp.json()

    choices = data.get("choices", [])
    if choices:
        return choices[0].get("message", {}).get("content", "")
    return ""


# ============================================================
# Search Providers (inline — reuse websearch patterns)
# ============================================================

async def search_web(query: str, max_results: int = 5) -> list[dict]:
    """Search using configured provider."""
    async with httpx.AsyncClient(timeout=SEARCH_TIMEOUT) as client:
        if SEARCH_PROVIDER == "searxng":
            resp = await client.get(
                f"{SEARXNG_URL}/search",
                params={"q": query, "format": "json", "pageno": 1},
            )
            resp.raise_for_status()
            results = resp.json().get("results", [])[:max_results]
            return [{"title": r.get("title", ""), "url": r.get("url", ""), "snippet": r.get("content", "")} for r in results]
        else:  # tavily
            resp = await client.post(
                "https://api.tavily.com/search",
                json={"query": query, "max_results": max_results, "api_key": TAVILY_API_KEY},
            )
            resp.raise_for_status()
            results = resp.json().get("results", [])[:max_results]
            return [{"title": r.get("title", ""), "url": r.get("url", ""), "snippet": r.get("content", "")} for r in results]


def format_search_results(results: list[dict]) -> tuple[str, dict[str, str]]:
    """Format results with reference tags (REF:G1, REF:G2, ...)."""
    url_map = {}
    lines = []
    for i, r in enumerate(results, 1):
        ref = f"REF:G{i}"
        url_map[ref] = r["url"]
        lines.append(f"[{ref}] {r['title']}\n{r['snippet']}")
    return "\n\n".join(lines), url_map


# ============================================================
# Prompts (ported from ai_karin/src/prompts.rs)
# ============================================================

def query_expansion_prompt(query: str) -> str:
    return f"""You are a search engineer. Generate 3 diverse, specific search queries to find comprehensive information about the user's question.

<user_query>
{query}
</user_query>

Output format (queries only, one per line):
1. query1
2. query2
3. query3"""


def extraction_prompt(query: str, data: str, feedback: str | None = None) -> str:
    base = f"""Extract key facts related to "{query}" from the following data. List them as bullet points.

Rules:
1. Include reference tags [REF:G1] etc. at the end of each fact.
2. Only extract facts. No greetings or speculation.
3. If no relevant information is found, output "No relevant information found."

<data>
{data}
</data>"""
    if feedback:
        base += f"\n\nPrevious evaluation feedback (address these issues):\n{feedback}"
    return base


def evaluation_prompt(query: str, extraction: str) -> str:
    return f"""Evaluate the quality of extracted facts for the given query.

<query>{query}</query>
<extracted_facts>{extraction}</extracted_facts>

Scoring criteria (total 10):
1. Information completeness (5 points): Does the extraction answer the query?
2. Usefulness (3 points): Is the information helpful even if incomplete?
3. References (2 points): Are [REF:] tags properly included?

Pass threshold: 6 points.

Output ONLY this JSON (no code blocks):
{{"score": <0-10>, "feedback": "<improvement needed or 'none'>", "missing_links": "<what to search next or 'none'>"}}"""


def refine_query_prompt(previous_query: str, feedback: str) -> str:
    return f"""You are a search expert using multi-hop reasoning. Generate a better search query.

Previous query: {previous_query}
Current gaps: {feedback}

Generate ONE new, more specific search query to fill the gaps. Output the query only, nothing else."""


def synthesis_prompt(query: str, research_data: str) -> str:
    return f"""You are a research analyst. Synthesize the following research findings into a comprehensive, well-structured answer.

<query>{query}</query>
<research_findings>{research_data}</research_findings>

Instructions:
1. Provide accurate, factual information based solely on the research findings.
2. Include reference tags [REF:G1] etc. where appropriate.
3. Structure the answer clearly with sections if needed.
4. If information is incomplete, state what is known and what remains unclear.
5. Do not fabricate information not present in the findings."""


# ============================================================
# RAG Engine — Self-evaluating research loop
# ============================================================

async def expand_query(query: str) -> list[str]:
    """Expand a query into multiple sub-queries using LLM."""
    try:
        result = await call_llm(EVALUATE_PROVIDER, query_expansion_prompt(query))
        queries = []
        for line in result.strip().split("\n"):
            line = line.strip()
            if line:
                # Remove numbering prefix (e.g., "1. ", "2. ")
                cleaned = re.sub(r"^\d+\.\s*", "", line).strip()
                if cleaned:
                    queries.append(cleaned)
        return queries if queries else [query]
    except Exception:
        return [query]


def parse_evaluation(content: str) -> tuple[int, str, str]:
    """Parse LLM evaluation response."""
    try:
        data = json.loads(content)
        return (
            int(data.get("score", 0)),
            data.get("feedback", "none"),
            data.get("missing_links", "none"),
        )
    except (json.JSONDecodeError, ValueError):
        # Fallback: extract from text
        score = 0
        feedback = "none"
        missing = "none"
        for line in content.split("\n"):
            if "score" in line.lower():
                nums = re.findall(r"\d+", line)
                if nums:
                    score = int(nums[0])
            elif "feedback" in line.lower():
                feedback = line.split(":", 1)[-1].strip() if ":" in line else feedback
            elif "missing" in line.lower():
                missing = line.split(":", 1)[-1].strip() if ":" in line else missing
        return score, feedback, missing


async def rag_loop(query: str) -> tuple[str, dict[str, str], dict]:
    """Core RAG loop: Search → Extract → Evaluate → Refine."""
    url_map: dict[str, str] = {}
    all_data = ""
    current_query = query
    stats = {"attempts": 0, "final_score": 0, "queries_used": []}

    for attempt in range(1, MAX_RETRIES + 1):
        stats["attempts"] = attempt
        stats["queries_used"].append(current_query)

        # 1. Search
        results = await search_web(current_query)
        formatted, new_urls = format_search_results(results)
        url_map.update(new_urls)

        if all_data:
            all_data += "\n---\n" + formatted
        else:
            all_data = formatted

        # 2. Extract (fast model)
        feedback_text = None if attempt == 1 else stats.get("last_feedback")
        extraction = await call_llm(
            EXTRACT_PROVIDER,
            extraction_prompt(query, all_data, feedback_text),
        )

        # 3. Evaluate (reasoning model)
        eval_result = await call_llm(EVALUATE_PROVIDER, evaluation_prompt(query, extraction))
        score, feedback, missing = parse_evaluation(eval_result)
        stats["final_score"] = score
        stats["last_feedback"] = feedback

        if score >= PASS_SCORE or attempt >= MAX_RETRIES:
            return extraction, url_map, stats

        # 4. Refine query (multi-hop)
        recovery_context = feedback
        if missing and missing.lower() not in ("none", "なし"):
            recovery_context = f"{feedback} (investigate further: {missing})"

        try:
            refined = await call_llm(EVALUATE_PROVIDER, refine_query_prompt(current_query, recovery_context))
            current_query = refined.strip().strip('"')
        except Exception:
            return extraction, url_map, stats

    return extraction, url_map, stats


async def deep_research(query: str) -> tuple[str, dict[str, str], dict]:
    """Full deep research pipeline."""
    # 1. Expand query
    expanded = await expand_query(query)

    # 2. RAG loop on primary query
    extraction, url_map, stats = await rag_loop(expanded[0])
    stats["expanded_queries"] = expanded

    # 3. Synthesize (high-quality model)
    synthesis = await call_llm(
        SYNTHESIZE_PROVIDER,
        synthesis_prompt(query, extraction),
        system="You are a thorough research analyst. Provide well-structured, accurate answers based on the provided research findings.",
    )

    return synthesis, url_map, stats


# ============================================================
# MCP Server
# ============================================================

server = Server("cloto-mcp-research")


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="deep_research",
            description=(
                "Perform deep research on a topic using agentic RAG with self-evaluation. "
                "Expands the query, searches the web, extracts and evaluates information quality, "
                "and refines the search if needed (multi-hop reasoning). Returns a synthesized "
                "research report with source references [REF:G1] etc."
            ),
            inputSchema={
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The research question or topic to investigate",
                    },
                    "agent_id": {
                        "type": "string",
                        "description": "The requesting agent's ID (for context)",
                    },
                },
                "required": ["query"],
            },
        ),
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "deep_research":
        return await handle_deep_research(arguments)
    return [TextContent(type="text", text=json.dumps({"error": f"Unknown tool: {name}"}))]


async def handle_deep_research(arguments: dict) -> list[TextContent]:
    query = arguments.get("query", "")
    if not query.strip():
        return [TextContent(type="text", text=json.dumps({"error": "Empty query"}))]

    try:
        synthesis, url_map, stats = await deep_research(query)

        response = {
            "result": synthesis,
            "sources": url_map,
            "stats": {
                "attempts": stats["attempts"],
                "final_score": stats["final_score"],
                "queries_used": stats["queries_used"],
                "expanded_queries": stats.get("expanded_queries", []),
            },
        }
        return [TextContent(type="text", text=json.dumps(response, ensure_ascii=False))]
    except Exception as e:
        return [TextContent(type="text", text=json.dumps({
            "error": f"Research failed: {e}",
            "query": query,
        }))]


# ============================================================
# Entry Point
# ============================================================

async def main():
    from mcp.server.stdio import stdio_server
    async with stdio_server() as (read_stream, write_stream):
        await server.run(read_stream, write_stream, server.create_initialization_options())


if __name__ == "__main__":
    asyncio.run(main())
