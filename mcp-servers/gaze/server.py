"""
Cloto MCP Server: Gaze Tracking
Webcam-based eye gaze detection via MediaPipe FaceLandmarker.

Provides AI agents with awareness of where the user is looking,
whether they are present at the screen, and attention status.
Camera capture and ML inference run in a background thread;
MCP tools return the latest result instantly.
"""

import asyncio
import os
import sys

sys.path.insert(0, os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")))

from common.mcp_utils import ToolRegistry, run_mcp_server
from gaze_engine import GazeEngine

# ============================================================
# Server setup
# ============================================================

registry = ToolRegistry("vision.gaze_webcam")
engine = GazeEngine()

# ============================================================
# Tool handlers
# ============================================================


@registry.tool(
    "start_tracking",
    "Start webcam camera capture and eye gaze tracking. "
    "Uses MediaPipe FaceLandmarker for iris detection. "
    "Runs continuously in background until stopped.",
    {"type": "object", "properties": {}, "required": []},
)
async def handle_start_tracking(arguments: dict) -> dict:
    result = await asyncio.get_event_loop().run_in_executor(None, engine.start)
    return {
        "status": result,
        "message": {
            "started": "Gaze tracking started. Camera is now active.",
            "already_running": "Gaze tracking is already running.",
        }.get(result, result),
    }


@registry.tool(
    "stop_tracking",
    "Stop gaze tracking and release the camera.",
    {"type": "object", "properties": {}, "required": []},
)
async def handle_stop_tracking(arguments: dict) -> dict:
    result = await asyncio.get_event_loop().run_in_executor(None, engine.stop)
    return {
        "status": result,
        "message": {
            "stopped": "Gaze tracking stopped. Camera released.",
            "not_running": "Gaze tracking was not running.",
        }.get(result, result),
    }


@registry.tool(
    "get_gaze",
    "Get the current gaze direction as normalized coordinates. "
    "Returns gaze_x [0-1] (0=left, 1=right) and gaze_y [0-1] "
    "(0=up, 1=down). Tracking must be started first.",
    {"type": "object", "properties": {}, "required": []},
)
async def handle_get_gaze(arguments: dict) -> dict:
    if not engine.is_running:
        err = engine.error
        return {"error": err or "Tracker not running. Call start_tracking first."}

    gaze = engine.get_gaze()
    return {
        "gaze_x": round(gaze.gaze_x, 4),
        "gaze_y": round(gaze.gaze_y, 4),
        "face_detected": gaze.face_detected,
        "confidence": round(gaze.confidence, 2),
        "timestamp": round(gaze.timestamp, 3),
    }


@registry.tool(
    "is_user_present",
    "Check if a user face is currently detected by the camera. "
    "Useful for attention monitoring and presence detection.",
    {"type": "object", "properties": {}, "required": []},
)
async def handle_is_user_present(arguments: dict) -> dict:
    if not engine.is_running:
        return {"error": "Tracker not running. Call start_tracking first."}

    gaze = engine.get_gaze()
    return {
        "present": gaze.face_detected,
        "confidence": round(gaze.confidence, 2),
    }


@registry.tool(
    "get_tracker_status",
    "Get the operational status of the gaze tracker: "
    "whether it is running, current FPS, camera resolution, "
    "and face detection state.",
    {"type": "object", "properties": {}, "required": []},
)
async def handle_get_tracker_status(arguments: dict) -> dict:
    return engine.get_status()


# ============================================================
# Entry point
# ============================================================


if __name__ == "__main__":
    print("Cloto MCP Gaze Server starting...", file=sys.stderr)
    asyncio.run(run_mcp_server(registry))
