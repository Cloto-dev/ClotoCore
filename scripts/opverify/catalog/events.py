"""Events domain — publish an event and prove it lands in the event history.

We publish a ``GazeUpdated`` event (a side-effect-free variant that does not
trigger the agentic loop / any LLM call), tagged with a sentinel coordinate,
then assert it reappears in ``GET /api/history``. Only MessageReceived /
VisionUpdated / GazeUpdated are accepted from external callers (others 403).
"""

from __future__ import annotations

from . import Operation, RunContext, register

_SENTINEL_X = 424242


@register
class EventsPublish(Operation):
    domain = "events"
    name = "publish"
    covers = ["POST /api/events/publish", "GET /api/history"]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        published = c.post(
            "/api/events/publish",
            body={
                "type": "GazeUpdated",
                "data": {
                    "x": _SENTINEL_X,
                    "y": 7,
                    "confidence": 0.99,
                    "fixated": True,
                },
            },
        )
        history = c.get("/api/history")
        found = False
        if isinstance(history, list):
            for ev in history:
                if ev.get("type") == "GazeUpdated":
                    d = ev.get("data") or {}
                    if d.get("x") == _SENTINEL_X:
                        found = True
                        break
        return {"published": published, "found_in_history": found, "history_len": len(history) if isinstance(history, list) else None}

    def assert_success(self, ctx: RunContext, result):
        assert result["found_in_history"], (
            "published GazeUpdated event not found in /api/history "
            f"(history_len={result['history_len']})"
        )
