"""Cron domain (Layer 2: Autonomous Trigger) — full job lifecycle driven to
success: create → appears in list → toggle off → toggle on → run-now
(dispatched) → delete → gone.

``run-now`` dispatches the job's message to the agent immediately; we assert
only that the dispatch was *accepted* (it returns before the agentic loop
finishes), so the op stays fast and does not couple to LLM latency.
"""

from __future__ import annotations

from . import Operation, RunContext, register

_AGENT = "agent.cloto_default"
_NAME = "opverify-cron-probe"


def _find(jobs, job_id):
    return next((j for j in jobs if j.get("id") == job_id), None)


@register
class CronLifecycle(Operation):
    domain = "cron"
    name = "lifecycle"
    covers = [
        "POST /api/cron/jobs",
        "GET /api/cron/jobs",
        "POST /api/cron/jobs/{id}/toggle",
        "POST /api/cron/jobs/{id}/run",
        "DELETE /api/cron/jobs/{id}",
    ]
    phase0 = False

    def drive(self, ctx: RunContext):
        c = ctx.client
        created = c.post(
            "/api/cron/jobs",
            body={
                "agent_id": _AGENT,
                "name": _NAME,
                "schedule_type": "interval",
                "schedule_value": "3600",  # seconds (min 60)
                "message": "opverify cron probe — no action needed",
            },
        )
        job_id = created["id"]
        ctx.scratch["cron_job_id"] = job_id

        listed = c.get("/api/cron/jobs")
        jobs = listed.get("jobs") if isinstance(listed, dict) else listed
        present = _find(jobs or [], job_id) is not None

        off = c.post(f"/api/cron/jobs/{job_id}/toggle", body={"enabled": False})
        on = c.post(f"/api/cron/jobs/{job_id}/toggle", body={"enabled": True})
        # dispatch immediately; accepted == 200 (loop runs async in background).
        c.post(f"/api/cron/jobs/{job_id}/run")

        c.delete(f"/api/cron/jobs/{job_id}")
        after = c.get("/api/cron/jobs")
        after_jobs = after.get("jobs") if isinstance(after, dict) else after
        gone = _find(after_jobs or [], job_id) is None
        ctx.scratch.pop("cron_job_id", None)

        return {
            "job_id": job_id,
            "present": present,
            "off": off,
            "on": on,
            "gone": gone,
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["job_id"], "no job id from create"
        assert result["present"], "created cron job not found in list"
        assert result["off"].get("enabled") is False, (
            f"toggle-off did not report disabled: {result['off']!r}"
        )
        assert result["on"].get("enabled") is True, (
            f"toggle-on did not report enabled: {result['on']!r}"
        )
        assert result["gone"], "cron job still present after delete"

    def teardown(self, ctx: RunContext):
        job_id = ctx.scratch.pop("cron_job_id", None)
        if job_id:
            try:
                ctx.client.delete(f"/api/cron/jobs/{job_id}")
            except Exception:
                pass
