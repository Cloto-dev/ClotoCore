"""Self-test of the assessment cache (#236). Run:
``python -m scripts.opverify.visual.assessor_cache_selftest`` (exit 0 = passed).

Proves the memoization keys on (frame, question) and — the point — that a
poll-until-visible checkpoint over a frozen screen invokes the expensive inner
(live) assessor exactly once instead of once per poll.
"""

from __future__ import annotations

import sys

from .assessor_cache import CachingAssessor
from .interfaces import VisionAssessment
from .settle import poll_until_visible
from .stub import FakeClock, ScriptedScreen, frame


class CountingVision:
    def __init__(self, visible: bool = False):
        self.calls = 0
        self._v = visible

    def assess(self, f, q) -> VisionAssessment:
        self.calls += 1
        return VisionAssessment(visible=self._v, detail=f"call{self.calls}")


def scenario_memoize() -> None:
    inner = CountingVision(visible=True)
    c = CachingAssessor(inner)
    fa, fb = frame("a"), frame("b")

    r1 = c.assess(fa, "q")
    r2 = c.assess(fa, "q")  # same frame + question → cache hit
    assert inner.calls == 1, inner.calls
    assert r1 is r2, "cache must return the identical cached verdict"

    c.assess(fb, "q")  # different frame → miss
    assert inner.calls == 2, inner.calls
    c.assess(fa, "q2")  # same frame, different question → miss
    assert inner.calls == 3, inner.calls
    assert c.inner_calls == 3


def scenario_poll_frozen_screen() -> None:
    # A frozen, never-visible screen: poll_until_visible polls until timeout, but
    # the cached assessor calls the expensive inner exactly once.
    clock = FakeClock()
    inner = CountingVision(visible=False)
    c = CachingAssessor(inner)
    screen = ScriptedScreen([frame("spinner")])  # every grab is the same frame

    f, a, appeared = poll_until_visible(
        screen,
        c,
        "is the reply visible?",
        timeout=5.0,
        interval=0.5,
        now=clock.now,
        sleep=clock.sleep,
    )
    assert appeared is False, "a frozen not-visible screen must time out"
    assert inner.calls == 1, f"frozen screen must assess once, got {inner.calls}"


def scenario_poll_changing_screen() -> None:
    # A changing screen (spinner then reply): the cache does not hide the change —
    # each distinct frame is assessed, and the reply is seen.
    clock = FakeClock()

    class MapVision:
        def __init__(self):
            self.calls = 0

        def assess(self, fr, q):
            self.calls += 1
            return VisionAssessment(
                visible=fr.fingerprint == frame("reply").fingerprint
            )

    inner = MapVision()
    c = CachingAssessor(inner)
    screen = ScriptedScreen([frame("spin"), frame("spin"), frame("reply")])
    f, a, appeared = poll_until_visible(
        screen,
        c,
        "visible?",
        timeout=5.0,
        interval=0.5,
        now=clock.now,
        sleep=clock.sleep,
    )
    assert appeared is True, "the reply must be seen"
    # two distinct frames assessed (spin cached across its repeat, then reply)
    assert inner.calls == 2, f"distinct frames only, got {inner.calls}"


def main() -> int:
    scenarios = [
        scenario_memoize,
        scenario_poll_frozen_screen,
        scenario_poll_changing_screen,
    ]
    for sc in scenarios:
        sc()
        print(f"  ok  {sc.__name__}")
    print(
        f"assessor-cache selftest: {len(scenarios)}/{len(scenarios)} scenarios passed"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
