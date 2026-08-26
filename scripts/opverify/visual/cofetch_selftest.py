"""Self-test of the grab + liveness co-fetch fast path — proves the
one-round-trip fusion without a real VM. Run:
``python -m scripts.opverify.visual.cofetch_selftest`` (exit 0 = all passed).

It asserts three things:
  1. the composite stream splits back into byte-identical (PNG, health) — even
     when the PNG payload happens to contain the delimiter bytes (rfind);
  2. a checkpoint (grab then health probe) costs exactly ONE ssh round trip, and
     the probe reads the co-fetched body (no second call);
  3. an unreachable kernel (empty health body) is a False gate, and a probe with
     no prior grab falls back to a standalone request.
"""

from __future__ import annotations

import sys

from . import backends_vm as B


def _fake_run_recorder():
    """Return (calls, run) where run mimics backends_vm._run: it records every
    remote command and returns a synthetic composite/plain body."""
    calls = []

    def run(remote, *, stdin=None, timeout=None):
        calls.append(remote)
        if "-w " in remote and "--next" in remote:  # composite grab+health
            png = calls_png
            return png + B._DELIM_BYTES + b'{"status": "ok"}'
        return b'{"status": "ok"}'  # standalone health fallback

    return calls, run


# a PNG-ish payload that deliberately embeds the delimiter to stress rfind
calls_png = b"\x89PNG\r\n" + B._DELIM_BYTES + b"fake-image-tail\x00\x01\x02"


def scenario_split_roundtrip() -> None:
    health = b'{"status": "ok"}'
    raw = calls_png + B._DELIM_BYTES + health
    png, got_health = B._split_composite(raw)
    assert png == calls_png, "PNG must survive intact (rfind picks the true tail)"
    assert got_health == health, got_health
    # No delimiter → whole payload is the frame, no health.
    p2, h2 = B._split_composite(b"just-a-frame")
    assert p2 == b"just-a-frame" and h2 == b"", (p2, h2)


def scenario_one_round_trip(monkey) -> None:
    calls, run = _fake_run_recorder()
    monkey(run)
    cell = B._CoFetchCell()
    screen = B.CompositeVmScreen(cell)
    probe = B.CoFetchHealthProbe(cell)

    frame = screen.grab()
    assert frame.data == calls_png, "grab must return the exact PNG bytes"
    assert len(calls) == 1, f"grab must be a single ssh call, got {len(calls)}"
    assert "--next" in calls[0] and "/grab" in calls[0], calls[0]

    ok = probe.check()
    assert ok is True, "co-fetched health body reports ok"
    assert len(calls) == 1, "probe must NOT add a round trip — it reads the cell"


def scenario_unreachable_and_fallback(monkey) -> None:
    # Empty health body (kernel unreachable) → False gate.
    cell = B._CoFetchCell()
    cell.put(b"")
    assert B.CoFetchHealthProbe(cell).check() is False

    # No grab yet → probe falls back to a standalone request (one call).
    calls, run = _fake_run_recorder()
    monkey(run)
    fresh = B._CoFetchCell()
    assert B.CoFetchHealthProbe(fresh).check() is True
    assert len(calls) == 1 and "--next" not in calls[0], calls


def main() -> int:
    orig = B._run
    try:
        scenario_split_roundtrip()
        print("  ok  scenario_split_roundtrip")

        def monkey(fn):
            B._run = fn

        scenario_one_round_trip(monkey)
        print("  ok  scenario_one_round_trip")
        scenario_unreachable_and_fallback(monkey)
        print("  ok  scenario_unreachable_and_fallback")
    finally:
        B._run = orig
    print("cofetch selftest: 3/3 scenarios passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
