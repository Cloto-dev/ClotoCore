"""Stub-driven self-test of the visual apex loop — proves the thesis without a
real GUI/VM/VLM. Run: ``python -m scripts.opverify.visual.selftest`` (exit 0 =
all scenarios behaved as designed).

It asserts the four dual-oracle outcomes and the tiering, headlined by the one
that justifies the whole apex: a step the **kernel confirms but the screen
never renders** is caught as a soft-tier ``FRONTEND_BUG`` — the defect class a
kernel-only or DOM-selector check is blind to.
"""

from __future__ import annotations

import os
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


def scenario_chat_journey_probe_discriminates() -> None:
    """The registered ``chat-render`` journey's kernel oracle must confirm the
    ASSISTANT answered — not merely that the nonce exists somewhere in the
    history.

    This is the trap the journey is built around: the user's own message is
    echoed back in ``/api/history`` and contains the nonce, so a probe matching
    the bare nonce passes even when the assistant never replied. The probe is
    therefore anchored to a ThoughtResponse whose content is *exactly* the
    nonce. Bodies below are compact JSON because that is what the kernel emits
    (verified against a live VM on 2026-08-03) — the probe is a substring test,
    so spacing is part of the contract.
    """
    from .run_vm import _JOURNEYS

    seen: list = []

    def fake_api_probe(path, want):
        seen.append((path, want))
        return object()

    spec = _JOURNEYS["chat-render"]
    recorded = spec.recorded
    journey = spec.factory(object(), fake_api_probe, None)

    names = [s.name for s in journey.steps]
    assert names[:5] == [
        "chat-view-rendered",
        "focus-chat-input",
        "type-nonce",
        "send",
        "await-reply-persisted",
    ], names
    assert names[-1] == "reply-rendered-without-scrolling", names
    # Nothing may sit between the wait and the assertion. The journey used to
    # scroll the newest turn into view first, because the thread did not follow
    # its own tail (bug-498); with that fixed the assertion covers auto-scroll
    # itself, and a re-introduced scroll step would silently take that away —
    # the assert would pass again on a pane that never moved.
    assert not any(n.startswith("scroll-") for n in names), names
    assert "point-at-transcript" not in names, names

    assessed = [s for s in journey.steps if s.vision_question]
    assert len(assessed) == len(recorded), (len(assessed), len(recorded))

    assert len(seen) == 1, seen
    path, want = seen[0]
    assert path == "/api/history", path

    # Recover the nonce the journey minted, then build the two history bodies.
    nonce = want.split('"content":"', 1)[1].split('"', 1)[0]
    assert nonce.isalnum(), f"nonce must survive the JP-layout keyboard: {nonce!r}"

    user_echo_only = (
        '{"data":[{"data":{"content":"Reply with exactly this token and nothing'
        f' else: {nonce}","id":"abc"}},"type":"ChatMessage"}}]}}'
    )
    assert want not in user_echo_only, (
        "the probe passes on the user's own echoed message — it would report a "
        "reply that never came"
    )

    with_assistant_reply = (
        user_echo_only[:-2]
        + ',{"data":{"agent_id":"agent.cloto_default","auto_spoken":false,'
        f'"content":"{nonce}","engine_id":"cerebras"}},"type":"ThoughtResponse"}}]}}'
    )
    assert want in with_assistant_reply, "the probe misses a genuine reply"

    # A reply that merely mentions the nonce inside a longer answer is not an
    # exact echo, and must not satisfy the oracle either.
    chatty = with_assistant_reply.replace(
        f'"content":"{nonce}"', f'"content":"sure, the token is {nonce}"'
    )
    assert want not in chatty, "the probe accepts a non-exact echo"

    # …and the asymmetric case: a reply that *begins* with the nonce and then
    # keeps talking. Only the closing anchor after the nonce rejects this, so
    # without it the oracle would accept a model that ignored "and nothing
    # else" — the exact drift the journey is supposed to notice.
    trailing = with_assistant_reply.replace(
        f'"content":"{nonce}"', f'"content":"{nonce} — here you go"'
    )
    assert want not in trailing, (
        "the probe accepts a reply that starts with the nonce but continues"
    )

    # The visual assertion must come AFTER the backend has the reply, and the
    # wait must live on its own step: the driver samples kernel_probe once,
    # after the visual oracle returns, so pinning the wait to the visual step
    # would sample the kernel at whatever moment that oracle happened to
    # finish — instantly, under the canned recorded assessor, which is before
    # any reply can exist (observed as a spurious backend_or_hidden on a
    # healthy app, 2026-08-03).
    wait = next(s for s in journey.steps if s.name == "await-reply-persisted")
    assert wait.vision_question is None, "the wait step must not cost an assessment"
    probe = wait.kernel_probe
    assert hasattr(probe, "_timeout"), "the kernel oracle does not wait"
    assert probe._timeout >= 60.0, probe._timeout

    reply = journey.steps[-1]
    assert reply.kernel_probe is None, (
        "the rendering assertion must not re-sample the kernel — the wait step "
        "already established it, and one step should carry one assertion"
    )
    assert journey.steps.index(wait) < journey.steps.index(reply)

    class _LateProbe:
        def __init__(self, succeed_on):
            self.calls = 0
            self._succeed_on = succeed_on

        def check(self):
            self.calls += 1
            return self.calls >= self._succeed_on

    late = _LateProbe(succeed_on=3)
    waiting = type(probe)(late, timeout=60.0, interval=0.0)
    assert waiting.check() is True, "a reply that lands late must still count"
    assert late.calls == 3, late.calls

    never = _LateProbe(succeed_on=10**9)
    bounded = type(probe)(never, timeout=0.0, interval=0.0)
    assert bounded.check() is False, "the wait must be bounded, not infinite"
    assert never.calls == 1, never.calls


def scenario_targeted_step_resolves_and_scrolls():
    """A step that declares a target clicks where the element *is now*, and
    wheels toward it first when it is off screen.

    The coordinate-in-the-journey failure mode is not theoretical: a stale
    (455, 453) selected the widest scope on 2026-07-31, and a stale (455, 725)
    closed the modal on 2026-08-05. Both landed on something, which is why they
    were only caught later, elsewhere.
    """
    class _T:
        def __init__(self, y, in_viewport, off):
            self.text, self.role = "UNINSTALL", "button"
            self.x, self.y = 700, y
            self.width = self.height = 20
            self.enabled, self.in_viewport, self.off_screen = True, in_viewport, off

    class _Targeter:
        """Off screen below until wheeled at twice, then in view — and it moves,
        so a driver that cached the first answer would click the wrong place."""

        def __init__(self):
            self.calls = 0

        def find(self, contains, *, nth=0, require_enabled=False, exact=False):
            self.calls += 1
            if self.calls < 3:
                return _T(1300, False, "below")
            return _T(640, True, "")

    class _DriftingTargeter:
        """In view, but still moving — the pane is animating after the wheel.
        Settles on the third reading."""

        def __init__(self):
            self.ys = [500, 560, 640, 640, 640]
            self.i = -1

        def find(self, contains, *, nth=0, require_enabled=False, exact=False):
            self.i = min(self.i + 1, len(self.ys) - 1)
            return _T(self.ys[self.i], True, "")

    targeter = _Targeter()
    actuator = RecordingActuator()
    driver = VisualDriver(
        screen=ScriptedScreen([frame("a")]),
        actuator=actuator,
        assessor=ScriptedVision({}, default=True),
        targeter=targeter,
        sleep=lambda _s: None,
    )
    report = driver.run(
        J.Journey(
            name="t",
            steps=[
                J.Step(
                    name="press",
                    target=J.TargetSpec(contains=("uninstall",), require_enabled=True),
                    vision_question="?",
                    settle=False,
                )
            ],
        )
    )
    kinds = [a.kind for a in actuator.actions]
    assert kinds == ["scroll", "scroll", "click"], kinds
    assert all(a.amount == -600 for a in actuator.actions if a.kind == "scroll"), actuator.actions
    click_action = actuator.actions[-1]
    assert (click_action.x, click_action.y) == (700, 640), click_action
    assert report.verdict == "pass", report.as_dict()

    # A journey that declares targets with no targeter must fail loudly rather
    # than fall through to `action` (which defaults to noop — a step that does
    # nothing and then reports on whatever was already on screen).
    blind = VisualDriver(
        screen=ScriptedScreen([frame("a")]),
        actuator=RecordingActuator(),
        assessor=ScriptedVision({}, default=True),
        sleep=lambda _s: None,
    )
    out = blind.run(J.Journey(name="t", steps=[J.Step(name="press", target=J.TargetSpec(contains="x"))]))
    assert out.verdict == "fail", out.as_dict()
    assert "targeter" in (out.steps[0].error or ""), out.steps[0].error

    # A target that is in view but still drifting must not be clicked at the
    # position it had while the pane was animating: that click lands wherever
    # the pane finally stops — the backdrop, on 2026-08-05, which closed the
    # modal and made the app look like it had refused to uninstall.
    drifting = RecordingActuator()
    VisualDriver(
        screen=ScriptedScreen([frame("a")]),
        actuator=drifting,
        assessor=ScriptedVision({}, default=True),
        targeter=_DriftingTargeter(),
        sleep=lambda _s: None,
    ).run(
        J.Journey(
            name="t",
            steps=[J.Step(name="press", target=J.TargetSpec(contains="x"), settle=False)],
        )
    )
    clicks = [a for a in drifting.actions if a.kind == "click"]
    assert len(clicks) == 1, drifting.actions
    assert clicks[0].y == 640, f"clicked a position the pane had already left: {clicks[0]}"


def scenario_exact_match_separates_nested_names():
    """One control's name containing another's is not a corner case: the
    danger zone's card button reads "Uninstall ClotoCore" and the confirm
    dialog's reads "Uninstall", and they are on screen together. A substring
    match takes whichever the DOM lists first — on 2026-08-05 the one *behind*
    the modal — so the confirmation was never given and the run recorded the
    app "failing to exit"."""
    from .cdp import CdpTargeter, Target

    card = Target(text="UNINSTALL CLOTOCORE", role="button", x=1, y=1, width=1,
                  height=1, enabled=True, in_viewport=True)
    dialog = Target(text="アンインストール", role="button", x=2, y=2, width=1,
                    height=1, enabled=True, in_viewport=True)

    targeter = CdpTargeter.__new__(CdpTargeter)
    targeter.last_affordances = []
    targeter.affordances = lambda: [card, dialog]  # DOM order: card first

    loose = targeter.find(("アンインストール", "uninstall"))
    assert loose is card, "the substring match is what picked the wrong button"

    exact = targeter.find(("アンインストール", "uninstall"), exact=True)
    assert exact is dialog, exact

    # And the card is still addressable by the part the dialog's name lacks.
    assert targeter.find(("をアンインストール", "uninstall clotocore")) is card


def scenario_purge_journey_shape():
    """The outcome journey asserts the machine after the kernel is gone, and
    refuses the fixtures that would make it vacuous."""
    from .run_vm import DESTRUCTIVE_JOURNEYS, _JOURNEYS, _danger_zone_purge_journey

    assert "danger-zone-purge" in DESTRUCTIVE_JOURNEYS
    assert _JOURNEYS["danger-zone-purge"].recorded == [], (
        "a destructive journey must carry no recorded verdicts to replay"
    )

    def plan(entries, tier=4):
        return {
            "data": {
                "summary": {"entries": entries, "needs_elevation": False, "contains_secret": False},
                "plan": {"data_dir": "C:\\d", "tier": "everything"},
            }
        }

    os.environ["OPV_API_KEY"] = "k"
    journey = _danger_zone_purge_journey(object(), lambda p, w: object(), lambda p: plan(9))
    names = [s.name for s in journey.steps]
    # The three assertions that only exist past the point of no return.
    assert names[-3:] == ["the-app-ends", "the-helper-reports", "residue-sweep-is-zero"], names
    # The detector is exercised before its zero is used as evidence (bug-497).
    assert names[1] == "residue-present-before-purge", names
    assert journey.steps[1].kernel_probe.expect == "present"
    assert journey.steps[-1].kernel_probe.expect == "empty"
    # The key is typed, never named in the step or the visual question.
    key_step = next(s for s in journey.steps if s.name == "enter-the-admin-key")
    assert key_step.target.type_text == "k"
    assert "k" not in (key_step.vision_question or "").split(), key_step.vision_question

    # An empty plan is refused: a purge with nothing to purge passes trivially.
    try:
        _danger_zone_purge_journey(object(), lambda p, w: object(), lambda p: plan(0))
    except RuntimeError as e:
        assert "empty" in str(e), e
    else:
        raise AssertionError("an empty tier-4 plan must refuse to build the journey")

    del os.environ["OPV_API_KEY"]
    try:
        _danger_zone_purge_journey(object(), lambda p, w: object(), lambda p: plan(9))
    except RuntimeError as e:
        assert "OPV_API_KEY" in str(e), e
    else:
        raise AssertionError("gate 3 needs the admin key; building without it must fail")


def scenario_residue_sweep_reports_what_it_finds():
    """The sweep is a detector, and its two directions are not symmetric: an
    empty result is a pass only when `expect='empty'`."""
    from . import os_oracle

    found = ["vendor_key_not_empty"]
    os_oracle.run_powershell_json = lambda script, timeout=60.0: found

    after = os_oracle.ResidueSweep(data_dir="C:\\d", expect="empty")
    assert after.check() is False and "vendor_key_not_empty" in after.detail
    before = os_oracle.ResidueSweep(data_dir="C:\\d", expect="present")
    assert before.check() is True

    found = []
    assert os_oracle.ResidueSweep(data_dir="C:\\d", expect="empty").check() is True
    assert os_oracle.ResidueSweep(data_dir="C:\\d", expect="present").check() is False, (
        "a sweep that finds nothing before the purge means the detector has not "
        "been shown to work on this machine"
    )
    # ConvertTo-Json collapses a single-element array to a bare string.
    found = "arp_key"
    assert os_oracle.ResidueSweep(data_dir="C:\\d", expect="empty").check() is False


def scenario_purge_report_is_not_satisfied_by_nothing():
    """bug-499 left no report at all, and a report whose entries are all
    `absent` is the empty-fixture case — neither may pass."""
    from . import os_oracle

    report = {"entries": []}
    os_oracle.run_powershell_json = lambda script, timeout=60.0: report

    probe = os_oracle.PurgeReportClean(wait_s=0.0)
    report = None
    assert probe.check() is False and "no report" in probe.detail

    report = {"entries": [{"id": "db", "outcome": "absent"}]}
    probe = os_oracle.PurgeReportClean(wait_s=0.0)
    assert probe.check() is False and "none removed" in probe.detail

    report = {"entries": [{"id": "db", "outcome": "removed"}, {"id": "x", "outcome": "refused"}]}
    probe = os_oracle.PurgeReportClean(wait_s=0.0)
    assert probe.check() is False and "refused" in probe.detail

    report = {
        "entries": [{"id": "db", "outcome": "removed"}, {"id": "wal", "outcome": "absent"}]
    }
    probe = os_oracle.PurgeReportClean(wait_s=0.0)
    assert probe.check() is True and probe.detail == "1 removed / 1 absent"


def _kernel_state(
    *,
    healthy=True,
    setup_complete=True,
    plan_entries=11,
    providers=None,
    messages=0,
):
    """A fake `fetch_json` returning the shapes the kernel really serializes —
    measured on the Windows guest (2026-08-06), envelope included. A fixture
    check that passes against a hand-drawn shape and fails against the product's
    is worth less than no check at all."""

    def fetch(path: str) -> dict:
        if path == "/api/system/health":
            if not healthy:
                raise RuntimeError("connection refused")
            return {"data": {"status": "ok"}}
        if path == "/api/setup/status":
            return {"data": {"setup_complete": setup_complete, "uv_available": False}}
        if path.startswith("/api/system/uninstall/plan"):
            return {
                "data": {
                    "plan": {"data_dir": "C:\\d", "tier": "everything"},
                    "summary": {"entries": plan_entries, "total_bytes": 133977721},
                }
            }
        if path == "/api/llm/providers":
            return {"data": {"providers": providers if providers is not None else []}}
        if path.endswith("/messages"):
            return {
                "data": {
                    "has_more": False,
                    "messages": [{"id": i} for i in range(messages)],
                }
            }
        raise AssertionError(f"unexpected path {path}")

    return fetch


def scenario_fixture_refuses_the_vacuous_states() -> None:
    """The three states that produce a green run asserting nothing, each
    caught by name before step one.

    Every one of these was observed as a *pass*: a purge on a data-free
    install (all entries `absent`), a chat journey on an empty thread (every
    turn visible whatever the scroll logic does), and a main-window journey
    started on the onboarding carousel (eight targets failing as if they were
    eight product defects).
    """
    from . import fixtures as FX

    empty_plan = FX.verify("onboarded", _kernel_state(plan_entries=0))
    assert not empty_plan.ok
    assert [n for n, ok, _ in empty_plan.failures()] == ["purge-plan-has-entries"], (
        empty_plan.as_dict()
    )

    carousel = FX.verify("onboarded", _kernel_state(setup_complete=False))
    assert not carousel.ok
    assert "setup-complete" in [n for n, ok, _ in carousel.failures()]

    configured = [
        {"id": "cerebras", "has_key": True, "configured": True, "engine_status": "connected"}
    ]
    # Read the threshold off the fixture rather than pinning it here: raising
    # the required depth is a normal edit, and a test that hardcodes the old
    # number fails for a reason that has nothing to do with the behaviour it
    # is checking (which is exactly what happened while writing this one).
    depth = next(
        c for c in FX.FIXTURES["configured-chat"].checks
        if isinstance(c, FX.ThreadDepth)
    ).minimum

    short = FX.verify(
        "configured-chat", _kernel_state(providers=configured, messages=depth - 1)
    )
    assert not short.ok
    assert [n for n, ok, _ in short.failures()] == ["thread-depth"], short.as_dict()

    ok = FX.verify(
        "configured-chat", _kernel_state(providers=configured, messages=depth)
    )
    assert ok.ok, ok.as_dict()


def scenario_fixture_reports_every_failure() -> None:
    """One trip to the VM, not one per failure: checks keep running after the
    first one fails, and a dead kernel is a failure rather than a crash."""
    from . import fixtures as FX

    both = FX.verify(
        "configured-chat", _kernel_state(providers=[], messages=0)
    )
    names = [n for n, ok, _ in both.failures()]
    assert names == ["provider-ready:cerebras", "thread-depth"], both.as_dict()
    assert "not present" in dict((n, d) for n, _, d in both.results)[
        "provider-ready:cerebras"
    ]

    dead = FX.verify("onboarded", _kernel_state(healthy=False))
    assert not dead.ok
    assert dead.results[0][0] == "kernel-responds" and not dead.results[0][1]
    assert len(dead.results) == 3, "a dead kernel must not stop the other checks"


def scenario_fixture_checks_are_specific() -> None:
    """A provider that exists but is half-configured does not satisfy the
    fixture — `has_key` alone is the state a purge left behind, and an
    uninstalled engine still lists as a provider (`engine_status`)."""
    from . import fixtures as FX

    half = [{"id": "cerebras", "has_key": False, "configured": True, "engine_status": "connected"}]
    assert not FX.ProviderReady("cerebras").check(
        _kernel_state(providers=half)
    )[0]

    gone = [{"id": "cerebras", "has_key": True, "configured": True, "engine_status": "uninstalled"}]
    ok, detail = FX.ProviderReady("cerebras").check(_kernel_state(providers=gone))
    assert not ok and "uninstalled" in detail, detail
    # …unless the caller only needs the settings, not a live engine.
    assert FX.ProviderReady("cerebras", require_connected=False).check(
        _kernel_state(providers=gone)
    )[0]


def scenario_every_journey_declares_a_known_fixture() -> None:
    """A journey's declared start state has to name a fixture that exists.

    This is the ratchet: a typo, or a fixture deleted out from under a
    journey, fails here in CI instead of on the VM at the end of a rollback.
    """
    from . import fixtures as FX
    from .run_vm import _JOURNEYS

    for name, spec in _JOURNEYS.items():
        assert spec.fixture is None or spec.fixture in FX.FIXTURES, (
            f"journey {name!r} declares unknown fixture {spec.fixture!r}"
        )
    assert _JOURNEYS["danger-zone-purge"].fixture == "onboarded", (
        "the purge journey needs an install with data to remove"
    )
    assert _JOURNEYS["chat-render"].fixture == "configured-chat"
    # Every fixture that claims a snapshot must say what state it is, so an
    # operator reading the failure knows what they are being asked to restore.
    for name, fixture in FX.FIXTURES.items():
        assert fixture.summary and fixture.checks, name
        assert fixture.snapshot or fixture.build, name


def scenario_fixture_restore_is_never_implicit() -> None:
    """Unknown fixtures are an error, not a silent pass, and a fixture with no
    snapshot refuses to roll back rather than inventing a name for `qm`."""
    from . import fixtures as FX

    unknown = FX.verify("no-such-fixture", _kernel_state())
    assert not unknown.ok and "unknown fixture" in unknown.error

    handmade = FX.Fixture(
        name="handmade", summary="s", checks=[FX.KernelResponds()], build="by hand"
    )
    FX.FIXTURES["handmade"] = handmade
    try:
        FX.rollback("handmade")
    except RuntimeError as e:
        assert "no snapshot" in str(e), e
    else:
        raise AssertionError("rollback invented a snapshot for a hand-built fixture")
    finally:
        del FX.FIXTURES["handmade"]

    text = FX.remedy("onboarded")
    assert "DISCARDS" in text, "the remedy must say what a rollback costs"
    assert FX.FIXTURES["onboarded"].snapshot in text


def scenario_remedy_survives_an_unset_vm_id() -> None:
    """The remedy for an unmet fixture must print without OPV_VM_ID.

    It is printed *because* the run could not go ahead, so it cannot have
    preconditions the run itself lacks. OPV_VM_ID is needed only to roll back,
    and reading it while composing the text turned an unmet fixture into a
    traceback — the guidance was replaced by a stack trace at exactly the
    moment it was being asked for (measured 2026-08-31, driving chat-render).

    This scenario clears the variable deliberately: the module preamble does
    `setdefault("OPV_VM_ID", "0")`, so every other scenario runs with it set
    and none of them could have seen this.
    """
    import os

    from . import fixtures as FX

    saved = os.environ.pop("OPV_VM_ID", None)
    try:
        for name, fixture in FX.FIXTURES.items():
            if not fixture.snapshot:
                continue
            text = FX.remedy(name)
            assert fixture.snapshot in text, name
            assert "OPV_VM_ID" in text, (
                f"remedy for {name!r} must name the variable it could not read"
            )
    finally:
        if saved is not None:
            os.environ["OPV_VM_ID"] = saved


def scenario_probe_failure_is_not_state_absence() -> None:
    """A kernel that cannot be asked must not be reported as a kernel that
    answered "no" (bug-500).

    The measured incident: the admin routes answered 403, the envelope had no
    `providers` / `messages` keys, and `.get(..., [])` turned that into a
    machine with no engine and an empty thread. Every check failed by name,
    the fixture read as unmet, and the printed remedy was a rollback — which
    discards the state the harness had merely failed to see.
    """
    from . import fixtures as FX
    from .interfaces import ProbeUnavailable

    def denied(path: str) -> dict:
        # Unauthenticated routes still answer; the admin ones do not. This is
        # exactly the shape of a wrong OPV_API_KEY, and the reason
        # `setup-complete` passing proves nothing about the credential.
        if path in ("/api/system/health", "/api/setup/status"):
            return _kernel_state()(path)
        raise ProbeUnavailable("kernel answered 403 for " + path, status=403)

    report = FX.verify("configured-chat", denied)
    assert not report.ok, "an unanswerable probe cannot be a pass"
    assert report.error, "the report must carry WHY it could not be evaluated"
    assert "403" in report.error, report.error

    # The distinction has to survive into what the operator reads: the checks
    # that never ran must not accuse the machine of lacking the state.
    details = {n: d for n, _, d in report.results}
    assert "could not ask" in details["provider-ready:cerebras"], details
    assert "not present" not in details["provider-ready:cerebras"], details

    # And the reachable ones still report honestly — the failure is localized.
    assert dict((n, ok) for n, ok, _ in report.results)["kernel-responds"] is True

    # A genuinely absent state is still an ordinary failure, with no error set:
    # the fix must not turn every unmet fixture into "unknown".
    absent = FX.verify("configured-chat", _kernel_state(providers=[], messages=0))
    assert not absent.ok and not absent.error, absent.as_dict()


def scenario_unanswerable_probe_is_never_told_to_roll_back() -> None:
    """The two remedies must stay different. `remedy` offers a rollback because
    the state is known to be wrong; `probe_remedy` must not, because the state
    is not known at all — and `qm rollback` discards the live machine with no
    snapshot of "now" to return to."""
    from . import fixtures as FX

    unmet = FX.remedy("configured-chat")
    assert "--rollback-to-fixture" in unmet, "a known-unmet fixture keeps its remedy"

    unknown = FX.probe_remedy("kernel answered 403 for /api/llm/providers")
    assert "rollback" not in unknown.lower(), unknown
    assert "OPV_API_KEY" in unknown, "name the credential that is actually suspect"
    assert "UNKNOWN" in unknown, "say plainly that nothing was learned"


def _placeholder_vm_config() -> None:
    """Name a machine for the scenarios that only format commands.

    These tests build ssh/qm command strings; they never open a connection. The
    host config has no default (it would be the author's own machine), so the
    test supplies its own. The .invalid TLD can never resolve, so a scenario
    that ever did try to connect would fail loudly instead of reaching someone.
    """
    os.environ.setdefault("OPV_PVE_HOST", "selftest@pve.invalid")
    os.environ.setdefault("OPV_VM_USER", "selftest")
    os.environ.setdefault("OPV_VM_IP", "vm.invalid")
    os.environ.setdefault("OPV_VM_ID", "0")


def main() -> int:
    _placeholder_vm_config()
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
        scenario_chat_journey_probe_discriminates,
        scenario_targeted_step_resolves_and_scrolls,
        scenario_exact_match_separates_nested_names,
        scenario_purge_journey_shape,
        scenario_residue_sweep_reports_what_it_finds,
        scenario_purge_report_is_not_satisfied_by_nothing,
        scenario_fixture_refuses_the_vacuous_states,
        scenario_fixture_reports_every_failure,
        scenario_fixture_checks_are_specific,
        scenario_every_journey_declares_a_known_fixture,
        scenario_fixture_restore_is_never_implicit,
        scenario_remedy_survives_an_unset_vm_id,
        scenario_probe_failure_is_not_state_absence,
        scenario_unanswerable_probe_is_never_told_to_roll_back,
    ]
    for sc in scenarios:
        sc()
        print(f"  ok  {sc.__name__}")
    print(f"visual selftest: {len(scenarios)}/{len(scenarios)} scenarios passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
