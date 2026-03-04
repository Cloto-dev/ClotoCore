"""
Cloto MCP Server: CRON Job Management
Stateless MCP server that proxies to the kernel REST API (/api/cron/*).
Agents can create, list, delete, toggle, and manually trigger CRON jobs.
"""

import asyncio
import json
import logging
import os

import httpx
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

logger = logging.getLogger(__name__)

# ============================================================
# Configuration
# ============================================================

API_BASE = os.environ.get("CLOTO_API_URL", "http://127.0.0.1:8081")
API_KEY = os.environ.get("CLOTO_API_KEY", "")
HTTP_TIMEOUT = 15  # seconds

# ============================================================
# HTTP helpers
# ============================================================


def _headers() -> dict[str, str]:
    h = {"Content-Type": "application/json"}
    if API_KEY:
        h["X-API-Key"] = API_KEY
    return h


async def _api_get(path: str, params: dict | None = None) -> dict:
    async with httpx.AsyncClient(timeout=HTTP_TIMEOUT) as client:
        resp = await client.get(
            f"{API_BASE}{path}", headers=_headers(), params=params
        )
        resp.raise_for_status()
        return resp.json()


async def _api_post(path: str, payload: dict | None = None) -> dict:
    async with httpx.AsyncClient(timeout=HTTP_TIMEOUT) as client:
        resp = await client.post(
            f"{API_BASE}{path}", headers=_headers(), json=payload or {}
        )
        resp.raise_for_status()
        return resp.json()


async def _api_delete(path: str) -> dict:
    async with httpx.AsyncClient(timeout=HTTP_TIMEOUT) as client:
        resp = await client.delete(f"{API_BASE}{path}", headers=_headers())
        resp.raise_for_status()
        return resp.json()


# ============================================================
# Tool implementations
# ============================================================


async def do_create_cron_job(args: dict) -> dict:
    """POST /api/cron/jobs"""
    agent_id = args.get("agent_id", "")
    if not agent_id:
        return {"error": "agent_id is required"}

    payload = {
        "agent_id": agent_id,
        "name": args.get("name", ""),
        "schedule_type": args.get("schedule_type", ""),
        "schedule_value": args.get("schedule_value", ""),
        "message": args.get("message", ""),
    }
    if args.get("engine_id"):
        payload["engine_id"] = args["engine_id"]
    if args.get("max_iterations") is not None:
        payload["max_iterations"] = args["max_iterations"]

    return await _api_post("/api/cron/jobs", payload)


async def do_list_cron_jobs(args: dict) -> dict:
    """GET /api/cron/jobs[?agent_id=X]"""
    params = {}
    if args.get("agent_id"):
        params["agent_id"] = args["agent_id"]
    return await _api_get("/api/cron/jobs", params or None)


async def do_delete_cron_job(args: dict) -> dict:
    """DELETE /api/cron/jobs/:id"""
    job_id = args.get("job_id", "")
    if not job_id:
        return {"error": "job_id is required"}
    return await _api_delete(f"/api/cron/jobs/{job_id}")


async def do_toggle_cron_job(args: dict) -> dict:
    """POST /api/cron/jobs/:id/toggle"""
    job_id = args.get("job_id", "")
    if not job_id:
        return {"error": "job_id is required"}
    enabled = args.get("enabled")
    if enabled is None:
        return {"error": "enabled (bool) is required"}
    return await _api_post(f"/api/cron/jobs/{job_id}/toggle", {"enabled": enabled})


async def do_run_cron_job(args: dict) -> dict:
    """POST /api/cron/jobs/:id/run"""
    job_id = args.get("job_id", "")
    if not job_id:
        return {"error": "job_id is required"}
    return await _api_post(f"/api/cron/jobs/{job_id}/run")


# ============================================================
# MCP Server
# ============================================================

server = Server("cloto-mcp-cron")


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="create_cron_job",
            description=(
                "Create a scheduled CRON job for the current agent. "
                "The job will automatically send the specified message to the agent "
                "on the defined schedule."
            ),
            inputSchema={
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent identifier (your own agent ID)",
                    },
                    "name": {
                        "type": "string",
                        "description": "Human-readable name for this job (e.g. 'Daily Report')",
                    },
                    "schedule_type": {
                        "type": "string",
                        "enum": ["interval", "cron", "once"],
                        "description": (
                            "Schedule type: "
                            "'interval' = repeat every N seconds (min 60), "
                            "'cron' = standard cron expression (e.g. '0 9 * * *'), "
                            "'once' = run once at a specific ISO 8601 datetime"
                        ),
                    },
                    "schedule_value": {
                        "type": "string",
                        "description": (
                            "Schedule value matching schedule_type: "
                            "seconds for interval, cron expression for cron, "
                            "ISO 8601 datetime for once"
                        ),
                    },
                    "message": {
                        "type": "string",
                        "description": "The prompt/message sent to the agent when the job fires",
                    },
                    "engine_id": {
                        "type": "string",
                        "description": "Optional: override the LLM engine (e.g. 'mind.deepseek'). Uses agent default if omitted.",
                    },
                    "max_iterations": {
                        "type": "integer",
                        "description": "Max conversation turns per execution (default: 8)",
                        "default": 8,
                    },
                },
                "required": ["agent_id", "name", "schedule_type", "schedule_value", "message"],
            },
        ),
        Tool(
            name="list_cron_jobs",
            description="List CRON jobs. Filter by agent_id to see only your own jobs.",
            inputSchema={
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent identifier to filter by (optional, omit to list all)",
                    },
                },
                "required": [],
            },
        ),
        Tool(
            name="delete_cron_job",
            description="Delete a CRON job by its ID.",
            inputSchema={
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "The CRON job ID to delete (e.g. 'cron.agent.karin.abc123')",
                    },
                },
                "required": ["job_id"],
            },
        ),
        Tool(
            name="toggle_cron_job",
            description="Enable or disable a CRON job without deleting it.",
            inputSchema={
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "The CRON job ID to toggle",
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "true to enable, false to disable",
                    },
                },
                "required": ["job_id", "enabled"],
            },
        ),
        Tool(
            name="run_cron_job_now",
            description="Trigger immediate execution of a CRON job (ignores schedule).",
            inputSchema={
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "The CRON job ID to trigger",
                    },
                },
                "required": ["job_id"],
            },
        ),
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    try:
        if name == "create_cron_job":
            result = await do_create_cron_job(arguments)
        elif name == "list_cron_jobs":
            result = await do_list_cron_jobs(arguments)
        elif name == "delete_cron_job":
            result = await do_delete_cron_job(arguments)
        elif name == "toggle_cron_job":
            result = await do_toggle_cron_job(arguments)
        elif name == "run_cron_job_now":
            result = await do_run_cron_job(arguments)
        else:
            result = {"error": f"Unknown tool: {name}"}

        return [TextContent(type="text", text=json.dumps(result))]
    except httpx.HTTPStatusError as e:
        body = e.response.text
        try:
            body = json.dumps(e.response.json())
        except Exception:
            pass
        return [
            TextContent(
                type="text",
                text=json.dumps({"error": f"API {e.response.status_code}: {body}"}),
            )
        ]
    except Exception as e:
        return [TextContent(type="text", text=json.dumps({"error": str(e)}))]


# ============================================================
# Entry point
# ============================================================


async def main():
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(name)s] %(levelname)s: %(message)s",
    )
    logger.info(
        "CRON MCP server starting (api=%s, key=%s)",
        API_BASE,
        "***" if API_KEY else "(none)",
    )

    async with stdio_server() as (read_stream, write_stream):
        await server.run(
            read_stream, write_stream, server.create_initialization_options()
        )


if __name__ == "__main__":
    asyncio.run(main())
