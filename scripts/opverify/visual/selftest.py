"""Stub-driven self-test of the visual apex loop — proves the thesis without a
real GUI/VM/VLM. Run: ``python -m scripts.opverify.visual.selftest`` (exit 0 =
all scenarios behaved as designed).

It asserts the four dual-oracle outcomes and the tiering, headlined by the one
that justifies the whole apex: a step the **kernel confirms but the screen
never renders** is caught as a soft-tier ``FRONTEND_BUG`` — the defect class a
kernel-only or DOM-selector check is blind to.
"""

from __future__ import annotations

import sys

from . import journey as J
from .dual_oracle import Diagnosis
from .driver import VisualDriver
from .interfaces import click, type_text
from .stub import (
    FakeClock,
    RecordingActuator,
    ScriptedKernel,
    ScriptedScreen,
    ScriptedVision,
    frame,
)

_Q = "is the assistant reply rendered and non-empty?"


def _driver(screen, vision):
    clock = FakeClock()
    return VisualDriver(
        screen, RecordingActuator(), vision, now=clock.now, sleep=clock.sleep
    )


def _chat_journey(kernel_ok, trigger=J.POLL_UNTIL_VISIBLE):
    return J.Journey(
        name="chat-render",
        steps=[
            J.Step(
                name="type-message",
                action=type_text("hello"),
                trigger=J.CHECKPOINT,
                vision_question=None,  # typing has no async outcome to see yet
            ),
            J.Step(
                name="send",
                action=click(100, 200),
                trigger=J.CHECKPOINT,
                vision_question=None,
            ),
            J.Step(
                name="reply-renders",
                trigger=trigger,
                vision_question=_Q,
                kernel_probe=ScriptedKernel(kernel_ok),
                poll_timeout=2.0,
            ),
        ],
    )


def _reply_step(report):
    return next(s for s in report.steps if s.name == "reply-renders")


def scenario_happy() -> None:
    # Reply both persists in the kernel and renders on screen.
    screen = ScriptedScreen([frame("compose"), frame("reply"), frame("reply")])
    vision = ScriptedVision({frame("reply").fingerprint: True})
    report = _driver(screen, vision).run(_chat_journey(kernel_ok=True))
    step = _reply_step(report)
    assert step.verdict.diagnosis is Diagnosis.AGREE_PASS, step.verdict
    assert report.verdict == "pass", report.as_dict()


def scenario_frontend_bug() -> None:
    # Kernel says the reply exists, but it never renders (stuck spinner):
    # poll_until_visible times out → visual_ok False, kernel_ok True.
    screen = ScriptedScreen([frame("spinner")])  # never changes to a reply
    vision = ScriptedVision({}, default=False)  # never sees the reply
    report = _driver(screen, vision).run(_chat_journey(kernel_ok=True))
    step = _reply_step(report)
    assert step.appeared is False, step
    assert step.verdict.diagnosis is Diagnosis.FRONTEND_BUG, step.verdict
    assert step.verdict.soft_fail and not step.verdict.hard_fail, step.verdict
    assert step.forensic is not None, "a broken step must retain a forensic frame"
    # Visual layer starts soft → the run warns (reported), does not hard-fail.
    assert report.verdict == "warn", report.as_dict()


def scenario_backend_fail() -> None:
    # The screen shows a reply but the kernel never confirms it (stale/faked UI).
    screen = ScriptedScreen([frame("reply"), frame("reply")])
    vision = ScriptedVision({frame("reply").fingerprint: True})
    report = _driver(screen, vision).run(_chat_journey(kernel_ok=False))
    step = _reply_step(report)
    assert step.verdict.diagnosis is Diagnosis.BACKEND_OR_HIDDEN, step.verdict
    assert step.verdict.hard_fail, step.verdict
    assert report.verdict == "fail", report.as_dict()


def scenario_agree_fail() -> None:
    # Neither the kernel nor the screen shows a reply — a genuine failure.
    screen = ScriptedScreen([frame("spinner")])
    vision = ScriptedVision({}, default=False)
    report = _driver(screen, vision).run(_chat_journey(kernel_ok=False))
    step = _reply_step(report)
    assert step.verdict.diagnosis is Diagnosis.AGREE_FAIL, step.verdict
    assert step.verdict.hard_fail, step.verdict
    assert report.verdict == "fail", report.as_dict()


def scenario_settle() -> None:
    # A checkpoint with a changing-then-stable UI must settle on the stable tail.
    screen = ScriptedScreen(
        [frame("f0"), frame("f1"), frame("f2"), frame("f2"), frame("f2")]
    )
    vision = ScriptedVision({frame("f2").fingerprint: True})
    journey = J.Journey(
        name="settle-check",
        steps=[
            J.Step(
                name="wait-stable",
                trigger=J.CHECKPOINT,
                vision_question=_Q,
                kernel_probe=ScriptedKernel(True),
            )
        ],
    )
    report = _driver(screen, vision).run(journey)
    assert report.steps[0].verdict.diagnosis is Diagnosis.AGREE_PASS, report.as_dict()


def main() -> int:
    scenarios = [
        scenario_happy,
        scenario_frontend_bug,
        scenario_backend_fail,
        scenario_agree_fail,
        scenario_settle,
    ]
    for sc in scenarios:
        sc()
        print(f"  ok  {sc.__name__}")
    print(f"visual selftest: {len(scenarios)}/{len(scenarios)} scenarios passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
