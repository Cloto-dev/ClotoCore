"""
Cloto MCP Server: CRON Job Management
Stateless MCP server that proxies to the kernel REST API (/api/cron/*).
Agents can create, list, delete, toggle, and manually trigger CRON jobs.
"""

import asyncio
import json
import logging
import os
import sys

import httpx

sys.path.insert(0, os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")))

from common.mcp_utils import ToolRegistry, run_mcp_server

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


def _wrap_http_error(fn):
    """Decorator to catch httpx.HTTPStatusError and return error dict."""
    async def wrapper(arguments: dict) -> dict:
        try:
            return await fn(arguments)
        except httpx.HTTPStatusError as e:
            body = e.response.text
            try:
                body = json.dumps(e.response.json())
            except Exception:
                pass
            return {"error": f"API {e.response.status_code}: {body}"}
    wrapper.__name__ = fn.__name__
    return wrapper


# ============================================================
# MCP Server
# ============================================================

registry = ToolRegistry("cloto-mcp-cron")


@registry.tool(
    "create_cron_job",
    "Create a scheduled CRON job for the current agent. "
    "The job will automatically send the specified message to the agent "
    "on the defined schedule.",
    {
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
            "hide_prompt": {
                "type": "boolean",
                "description": (
                    "Agent-speak mode: if true, the cron prompt is hidden from chat "
                    "and only the agent's response is displayed. "
                    "Default: false (prompt shown as user message)."
                ),
                "default": False,
            },
            "source_type": {
                "type": "string",
                "enum": ["user", "system"],
                "description": (
                    "Message source type for this CRON job. "
                    "'user' = messages appear in the creator's chat history as user messages. "
                    "'system' = messages use system identity (default). "
                    "Infer from conversation context which is appropriate."
                ),
                "default": "system",
            },
            "creator_user_id": {
                "type": "string",
                "description": (
                    "User ID for 'user' source type. Required when source_type='user'. "
                    "Infer from the current conversation context."
                ),
            },
            "creator_user_name": {
                "type": "string",
                "description": (
                    "User display name for 'user' source type. Required when source_type='user'."
                ),
            },
        },
        "required": ["agent_id", "name", "schedule_type", "schedule_value", "message"],
    },
)
@_wrap_http_error
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
    if args.get("hide_prompt"):
        payload["hide_prompt"] = True
    if args.get("source_type"):
        payload["source_type"] = args["source_type"]
    if args.get("creator_user_id"):
        payload["creator_user_id"] = args["creator_user_id"]
    if args.get("creator_user_name"):
        payload["creator_user_name"] = args["creator_user_name"]

    return await _api_post("/api/cron/jobs", payload)


@registry.tool(
    "list_cron_jobs",
    "List CRON jobs. Filter by agent_id to see only your own jobs.",
    {
        "type": "object",
        "properties": {
            "agent_id": {
                "type": "string",
                "description": "Agent identifier to filter by (optional, omit to list all)",
            },
        },
        "required": [],
    },
)
@_wrap_http_error
async def do_list_cron_jobs(args: dict) -> dict:
    """GET /api/cron/jobs[?agent_id=X]"""
    params = {}
    if args.get("agent_id"):
        params["agent_id"] = args["agent_id"]
    return await _api_get("/api/cron/jobs", params or None)


@registry.tool(
    "delete_cron_job",
    "Delete a CRON job by its ID.",
    {
        "type": "object",
        "properties": {
            "job_id": {
                "type": "string",
                "description": "The CRON job ID to delete (e.g. 'cron.agent.karin.abc123')",
            },
        },
        "required": ["job_id"],
    },
)
@_wrap_http_error
async def do_delete_cron_job(args: dict) -> dict:
    """DELETE /api/cron/jobs/:id"""
    job_id = args.get("job_id", "")
    if not job_id:
        return {"error": "job_id is required"}
    return await _api_delete(f"/api/cron/jobs/{job_id}")


@registry.tool(
    "toggle_cron_job",
    "Enable or disable a CRON job without deleting it.",
    {
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
)
@_wrap_http_error
async def do_toggle_cron_job(args: dict) -> dict:
    """POST /api/cron/jobs/:id/toggle"""
    job_id = args.get("job_id", "")
    if not job_id:
        return {"error": "job_id is required"}
    enabled = args.get("enabled")
    if enabled is None:
        return {"error": "enabled (bool) is required"}
    return await _api_post(f"/api/cron/jobs/{job_id}/toggle", {"enabled": enabled})


@registry.tool(
    "run_cron_job_now",
    "Trigger immediate execution of a CRON job (ignores schedule).",
    {
        "type": "object",
        "properties": {
            "job_id": {
                "type": "string",
                "description": "The CRON job ID to trigger",
            },
        },
        "required": ["job_id"],
    },
)
@_wrap_http_error
async def do_run_cron_job(args: dict) -> dict:
    """POST /api/cron/jobs/:id/run"""
    job_id = args.get("job_id", "")
    if not job_id:
        return {"error": "job_id is required"}
    return await _api_post(f"/api/cron/jobs/{job_id}/run")


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

    await run_mcp_server(registry)


if __name__ == "__main__":
    asyncio.run(main())
