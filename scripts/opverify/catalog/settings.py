"""Settings domain — the two kernel-wide switches the dashboard exposes.

Both are read-modify-read pairs backed by an in-process atomic plus a DB row,
so "took effect" means the *other* route observes the change, not that the
write returned 200. Both operations restore the value the instance booted
with, because everything after them in the catalog runs against the same
kernel: ``yolo`` decides whether MGP permission requests are auto-approved
(``permissions.decide`` depends on it being off) and
``max-cron-generation`` bounds cron recursion.

The refusals are asserted alongside the writes: ``max-cron-generation``
declares a ceiling of 6 in its own response, and a bound that is advertised
but not enforced is worse than no bound.
"""

from __future__ import annotations

from . import Operation, RunContext, register


@register
class SettingsYolo(Operation):
    """YOLO mode — auto-approve every MGP permission / sandboxed command.

    Asserted as a real toggle: set to the *opposite* of the booted value, read
    it back through ``GET`` (a separate handler reading a separate source: the
    live atomic, not the request body echoed back), then restore. Reading back
    the value we just sent from the same response would pass against a handler
    that stored nothing.
    """

    domain = "settings"
    name = "yolo"
    covers = ["GET /api/settings/yolo", "PUT /api/settings/yolo"]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        initial = c.get("/api/settings/yolo")
        booted = bool((initial or {}).get("enabled"))
        ctx.scratch["yolo_booted"] = booted

        flipped_resp = c.put("/api/settings/yolo", body={"enabled": not booted})
        flipped_read = c.get("/api/settings/yolo")
        c.put("/api/settings/yolo", body={"enabled": booted})
        restored_read = c.get("/api/settings/yolo")
        ctx.scratch.pop("yolo_booted", None)

        return {
            "booted": booted,
            "flipped_resp": flipped_resp,
            "flipped_read": flipped_read,
            "restored_read": restored_read,
        }

    def assert_success(self, ctx: RunContext, result):
        booted = result["booted"]
        assert (result["flipped_resp"] or {}).get("enabled") is (not booted), (
            f"PUT did not echo the new value: {result['flipped_resp']!r}"
        )
        assert (result["flipped_read"] or {}).get("enabled") is (not booted), (
            f"GET still reports {result['flipped_read']!r} after PUT set "
            f"enabled={not booted} — the toggle did not reach the live state"
        )
        assert (result["restored_read"] or {}).get("enabled") is booted, (
            f"failed to restore the booted value {booted}: "
            f"{result['restored_read']!r}"
        )

    def teardown(self, ctx: RunContext):
        booted = ctx.scratch.pop("yolo_booted", None)
        if booted is not None:
            try:
                ctx.client.put("/api/settings/yolo", body={"enabled": booted})
            except Exception:  # noqa: BLE001
                pass


@register
class SettingsMaxCronGeneration(Operation):
    """The cron recursion depth limit.

    ``GET`` answers ``{value, max}``; ``PUT`` accepts 0..=max and refuses
    anything above it. Both halves are asserted, and the value is restored.
    """

    domain = "settings"
    name = "max_cron_generation"
    covers = [
        "GET /api/settings/max-cron-generation",
        "PUT /api/settings/max-cron-generation",
    ]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        initial = c.get("/api/settings/max-cron-generation")
        booted = (initial or {}).get("value")
        ceiling = (initial or {}).get("max")
        ctx.scratch["max_cron_booted"] = booted

        # Pick a value inside the advertised range that differs from the
        # current one, so a handler that ignores the body cannot pass.
        target = 0 if booted != 0 else 1
        set_resp = c.put(
            "/api/settings/max-cron-generation", body={"value": target}
        )
        read_back = c.get("/api/settings/max-cron-generation")

        over_status, over_body = c.request_raw(
            "PUT",
            "/api/settings/max-cron-generation",
            body={"value": (ceiling or 6) + 1},
        )
        read_after_refusal = c.get("/api/settings/max-cron-generation")

        c.put("/api/settings/max-cron-generation", body={"value": booted})
        restored = c.get("/api/settings/max-cron-generation")
        ctx.scratch.pop("max_cron_booted", None)

        return {
            "booted": booted,
            "ceiling": ceiling,
            "target": target,
            "set_resp": set_resp,
            "read_back": read_back,
            "over_status": over_status,
            "over_body": over_body,
            "read_after_refusal": read_after_refusal,
            "restored": restored,
        }

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result["booted"], int), (
            f"GET returned no integer value: {result!r}"
        )
        assert isinstance(result["ceiling"], int) and result["ceiling"] > 0, (
            f"GET advertises no usable max: {result['ceiling']!r}"
        )
        assert (result["set_resp"] or {}).get("value") == result["target"], (
            f"PUT did not echo the new value: {result['set_resp']!r}"
        )
        assert (result["read_back"] or {}).get("value") == result["target"], (
            f"GET still reports {result['read_back']!r} after PUT set "
            f"value={result['target']}"
        )
        assert result["over_status"] == 400, (
            f"a value above the advertised max ({result['ceiling']}) was "
            f"accepted with HTTP {result['over_status']}: "
            f"{result['over_body'][:200]}"
        )
        assert (result["read_after_refusal"] or {}).get("value") == result["target"], (
            f"the refused write still changed the value: "
            f"{result['read_after_refusal']!r}"
        )
        assert (result["restored"] or {}).get("value") == result["booted"], (
            f"failed to restore the booted value {result['booted']}: "
            f"{result['restored']!r}"
        )

    def teardown(self, ctx: RunContext):
        booted = ctx.scratch.pop("max_cron_booted", None)
        if booted is not None:
            try:
                ctx.client.put(
                    "/api/settings/max-cron-generation", body={"value": booted}
                )
            except Exception:  # noqa: BLE001
                pass
