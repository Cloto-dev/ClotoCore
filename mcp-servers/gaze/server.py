"""
Cloto MCP Server: Gaze Tracking
Webcam-based eye gaze detection via MediaPipe FaceLandmarker.

Provides AI agents with awareness of where the user is looking,
whether they are present at the screen, and attention status.
Camera capture and ML inference run in a background thread;
MCP tools return the latest result instantly.
"""

import asyncio
import json
import sys

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

from gaze_engine import GazeEngine

# ============================================================
# Server setup
# ============================================================

server = Server("vision.gaze_webcam")
engine = GazeEngine()

# ============================================================
# Tool definitions
# ============================================================


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="start_tracking",
            description=(
                "Start webcam camera capture and eye gaze tracking. "
                "Uses MediaPipe FaceLandmarker for iris detection. "
                "Runs continuously in background until stopped."
            ),
            inputSchema={
                "type": "object",
                "properties": {},
                "required": [],
            },
        ),
        Tool(
            name="stop_tracking",
            description="Stop gaze tracking and release the camera.",
            inputSchema={
                "type": "object",
                "properties": {},
                "required": [],
            },
        ),
        Tool(
            name="get_gaze",
            description=(
                "Get the current gaze direction as normalized coordinates. "
                "Returns gaze_x [0-1] (0=left, 1=right) and gaze_y [0-1] "
                "(0=up, 1=down). Tracking must be started first."
            ),
            inputSchema={
                "type": "object",
                "properties": {},
                "required": [],
            },
        ),
        Tool(
            name="is_user_present",
            description=(
                "Check if a user face is currently detected by the camera. "
                "Useful for attention monitoring and presence detection."
            ),
            inputSchema={
                "type": "object",
                "properties": {},
                "required": [],
            },
        ),
        Tool(
            name="get_tracker_status",
            description=(
                "Get the operational status of the gaze tracker: "
                "whether it is running, current FPS, camera resolution, "
                "and face detection state."
            ),
            inputSchema={
                "type": "object",
                "properties": {},
                "required": [],
            },
        ),
    ]


# ============================================================
# Tool handlers
# ============================================================


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "start_tracking":
        result = await asyncio.get_event_loop().run_in_executor(
            None, engine.start
        )
        return [TextContent(type="text", text=json.dumps({
            "status": result,
            "message": {
                "started": "Gaze tracking started. Camera is now active.",
                "already_running": "Gaze tracking is already running.",
            }.get(result, result),
        }))]

    elif name == "stop_tracking":
        result = await asyncio.get_event_loop().run_in_executor(
            None, engine.stop
        )
        return [TextContent(type="text", text=json.dumps({
            "status": result,
            "message": {
                "stopped": "Gaze tracking stopped. Camera released.",
                "not_running": "Gaze tracking was not running.",
            }.get(result, result),
        }))]

    elif name == "get_gaze":
        if not engine.is_running:
            err = engine.error
            return [TextContent(type="text", text=json.dumps({
                "error": err or "Tracker not running. Call start_tracking first.",
            }))]

        gaze = engine.get_gaze()
        return [TextContent(type="text", text=json.dumps({
            "gaze_x": round(gaze.gaze_x, 4),
            "gaze_y": round(gaze.gaze_y, 4),
            "face_detected": gaze.face_detected,
            "confidence": round(gaze.confidence, 2),
            "timestamp": round(gaze.timestamp, 3),
        }))]

    elif name == "is_user_present":
        if not engine.is_running:
            return [TextContent(type="text", text=json.dumps({
                "error": "Tracker not running. Call start_tracking first.",
            }))]

        gaze = engine.get_gaze()
        return [TextContent(type="text", text=json.dumps({
            "present": gaze.face_detected,
            "confidence": round(gaze.confidence, 2),
        }))]

    elif name == "get_tracker_status":
        status = engine.get_status()
        return [TextContent(type="text", text=json.dumps(status))]

    else:
        return [TextContent(type="text", text=json.dumps({
            "error": f"Unknown tool: {name}",
        }))]


# ============================================================
# Entry point
# ============================================================


async def main():
    async with stdio_server() as (read_stream, write_stream):
        await server.run(read_stream, write_stream, server.create_initialization_options())


if __name__ == "__main__":
    print("Cloto MCP Gaze Server starting...", file=sys.stderr)
    asyncio.run(main())
