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
from .settle import settle
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


def scenario_settle_hashpoll() -> None:
    # settle polls the cheap change_probe (agent /grabhash) for stability and
    # grabs the full frame only ONCE, after the hash stabilizes — the settle
    # cost drops from N full grabs to N tiny hash polls + one grab.
    clock = FakeClock()
    grabbed = {"n": 0}
    settled = frame("stable")

    class OneGrab:
        def grab(self):
            grabbed["n"] += 1
            return settled

    hashes = iter(["h0", "h1", "h1"])  # stabilizes on the 2nd consecutive "h1"
    probe_calls = {"n": 0}

    def change_probe():
        probe_calls["n"] += 1
        return next(hashes)

    out = settle(
        OneGrab(),
        stable_needed=2,
        change_probe=change_probe,
        now=clock.now,
        sleep=clock.sleep,
    )
    assert out is settled, out
    assert grabbed["n"] == 1, f"exactly one full grab expected, got {grabbed['n']}"
    assert probe_calls["n"] == 3, f"three hash polls expected, got {probe_calls['n']}"


def scenario_roi_crop() -> None:
    """A step with an ROI hands the assessor a CROPPED frame while the run's
    own record keeps the full frame (#446). The crop implementation is
    injected so this exercises the driver's plumbing without Pillow; the
    assertions are asymmetric — the assessor must see the cropped bytes, and
    a failing step's forensic must still be the full frame."""
    from .interfaces import Frame

    full = frame("full")
    crops: list = []

    def fake_crop(f: Frame, roi: tuple) -> Frame:
        crops.append(roi)
        return Frame.of(b"cropped:" + f.data)

    class CapturingVision:
        def __init__(self) -> None:
            self.seen: list = []

        def assess(self, f: Frame, question: str):
            self.seen.append(f.data)
            from .interfaces import VisionAssessment

            return VisionAssessment(visible=False, detail="mismatch", defects=[])

    vision = CapturingVision()
    screen = ScriptedScreen([full])
    driver = VisualDriver(
        screen, RecordingActuator(), vision, crop=fake_crop, now=FakeClock().now,
        sleep=FakeClock().sleep,
    )
    report = driver.run(
        J.Journey(
            name="roi",
            steps=[
                J.Step(
                    name="counted",
                    trigger=J.CHECKPOINT,
                    settle=False,
                    vision_question="exactly 3 items?",
                    roi=(10, 20, 30, 40),
                )
            ],
        )
    )
    assert crops == [(10, 20, 30, 40)], crops
    assert vision.seen == [b"cropped:" + full.data], vision.seen
    # visible=False with no kernel probe → soft-tier failure; the forensic
    # must be the FULL frame, not the assessor's crop.
    assert report.steps[0].forensic is not None
    assert report.steps[0].forensic.data == full.data


def scenario_fetch_plan_envelope() -> None:
    """_fetch_plan must unwrap the kernel's {"data": ...} response envelope
    (handlers/response.rs ok_data wraps every handler payload). The first live
    apex run crashed because the fixture here matched the doc comment, not the
    write path — this fixture is the real wrapped shape."""
    from .run_vm import _fetch_plan

    wrapped = {"data": {"plan": {"data_dir": "X"}, "summary": {"entries": 1}}}
    body = _fetch_plan(lambda path: wrapped, 1)
    assert body["summary"]["entries"] == 1, body

    try:
        _fetch_plan(lambda path: {"data": {"error": "forbidden"}}, 1)
    except RuntimeError as e:
        assert "unexpected plan response" in str(e), e
    else:
        raise AssertionError("_fetch_plan accepted a payload without plan/summary")


def scenario_derived_questions() -> None:
    """The danger-zone questions embed the kernel plan's current counts and
    conditional clauses (runtime derivation — never an authored baseline).
    Fixtures are asymmetric on every axis (counts differ, elevation and secret
    flags come from different tiers) so a swapped or ignored field cannot pass
    by coincidence."""
    from .run_vm import derive_danger_zone_questions

    plan1 = {
        "plan": {"data_dir": "C:\\Users\\PC\\AppData\\Roaming\\cloto-system"},
        "summary": {"entries": 3, "needs_elevation": True, "contains_secret": False},
    }
    plan2 = {
        "plan": {"data_dir": "C:\\Users\\PC\\AppData\\Roaming\\cloto-system"},
        "summary": {"entries": 7, "needs_elevation": False, "contains_secret": True},
    }
    q1, q2 = derive_danger_zone_questions(plan1, plan2)
    assert "exactly 3 items" in q1, q1
    assert "administrator approval" in q1, q1
    assert "7 items" in q2, q2
    assert "cloto-system" in q2, q2
    assert "credentials will be destroyed" in q2, q2

    plan1["summary"] = {
        "entries": 1,
        "needs_elevation": False,
        "contains_secret": False,
    }
    plan2["summary"] = {
        "entries": 2,
        "needs_elevation": False,
        "contains_secret": False,
    }
    q1, q2 = derive_danger_zone_questions(plan1, plan2)
    assert "exactly 1 item," in q1, q1
    assert "administrator" not in q1, q1
    assert "2 items" in q2, q2
    assert "credentials" not in q2, q2


def main() -> int:
    scenarios = [
        scenario_happy,
        scenario_frontend_bug,
        scenario_backend_fail,
        scenario_agree_fail,
        scenario_settle,
        scenario_settle_hashpoll,
        scenario_roi_crop,
        scenario_fetch_plan_envelope,
        scenario_derived_questions,
    ]
    for sc in scenarios:
        sc()
        print(f"  ok  {sc.__name__}")
    print(f"visual selftest: {len(scenarios)}/{len(scenarios)} scenarios passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
