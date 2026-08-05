"""The orchestration loop — walk a journey, drive the GUI, judge each step
with the dual oracle, and produce a tiered report.

Trigger dispatch per step:

* ``CHECKPOINT`` / ``KERNEL_EVENT`` — settle, capture once, assess, kernel-check.
  (KERNEL_EVENT is a checkpoint whose cue is an emitted kernel event; the
  mechanics of *waiting* for the event live in the event source, not here.)
* ``POLL_UNTIL_VISIBLE`` — poll capture+assess until the thing appears or the
  bound elapses; ``appeared=False`` is the stuck/never-rendered signal.

On any non-clean step a **forensic** frame is retained (what the user would
have seen when it broke). The run verdict is tiered: any ``hard_fail`` ⇒
``fail``; else any ``soft_fail`` (a frontend divergence) ⇒ ``warn``; else
``pass``.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Callable, List, Optional

from . import journey as J
from .dual_oracle import Verdict, cross_check
from .interfaces import (
    Actuator,
    Frame,
    ScreenSource,
    VisionAssessment,
    VisionAssessor,
    click,
    scroll,
    type_text,
)
from .roi import crop_frame
from .settle import poll_until_visible, settle


@dataclass
class StepResult:
    name: str
    trigger: str
    verdict: Optional[Verdict]
    visual: Optional[VisionAssessment] = None
    appeared: Optional[bool] = None  # POLL_UNTIL_VISIBLE only
    forensic: Optional[Frame] = None  # retained only on a non-clean step
    error: Optional[str] = None


@dataclass
class RunReport:
    journey: str
    verdict: str  # "pass" | "warn" | "fail"
    steps: List[StepResult] = field(default_factory=list)

    def as_dict(self) -> dict:
        return {
            "journey": self.journey,
            "verdict": self.verdict,
            "steps": [
                {
                    "name": s.name,
                    "trigger": s.trigger,
                    "diagnosis": s.verdict.diagnosis.value if s.verdict else None,
                    "kernel_ok": s.verdict.kernel_ok if s.verdict else None,
                    "visual_ok": s.verdict.visual_ok if s.verdict else None,
                    "hard_fail": s.verdict.hard_fail if s.verdict else None,
                    "soft_fail": s.verdict.soft_fail if s.verdict else None,
                    "appeared": s.appeared,
                    "visual_detail": s.visual.detail if s.visual else None,
                    "defects": s.visual.defects if s.visual else None,
                    "forensic": s.forensic is not None,
                    "error": s.error,
                }
                for s in self.steps
            ],
        }


class VisualDriver:
    def __init__(
        self,
        screen: ScreenSource,
        actuator: Actuator,
        assessor: VisionAssessor,
        *,
        change_probe: Optional[Callable[[], str]] = None,
        targeter=None,
        crop: Callable[[Frame, tuple], Frame] = crop_frame,
        now: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], None] = time.sleep,
    ):
        self.screen = screen
        self.actuator = actuator
        self.assessor = assessor
        # Resolves a step's TargetSpec to a screen coordinate at the moment it
        # acts (see .cdp). None → a journey that declares targets fails loudly
        # rather than falling back to a coordinate that may not be there.
        self._targeter = targeter
        # Optional cheap settle signal (e.g. agent /grabhash) — halves settle
        # cost from N full grabs to N hash polls + one grab. None → grab-based.
        self._change_probe = change_probe
        # ROI implementation, injectable so the stub selftest can exercise the
        # plumbing without the orchestrator host's Pillow dependency.
        self._crop = crop
        self._now = now
        self._sleep = sleep

    def run(self, journey: J.Journey) -> RunReport:
        results: List[StepResult] = []
        for step in journey.steps:
            results.append(self._run_step(step))
        return RunReport(
            journey=journey.name,
            verdict=self._tier(results),
            steps=results,
        )

    # -- per-step -------------------------------------------------------
    def _act(self, step: J.Step) -> None:
        """Perform the step's input: a resolved target if it declares one, else
        its literal action."""
        if step.target is None:
            if step.action.kind != "noop":
                self.actuator.send(step.action)
            return
        if self._targeter is None:
            raise RuntimeError(
                f"step {step.name!r} declares a target but the driver has no "
                "targeter (was the app launched with cdp.debug_env()?)"
            )
        spec = step.target

        def resolve():
            return self._targeter.find(
                spec.contains,
                nth=spec.nth,
                require_enabled=spec.require_enabled,
                exact=spec.exact,
            )

        t = resolve()
        # Wheel toward it and look again. Scrolling by wheel rather than
        # scrollIntoView keeps the emulation a user's: the pane moves the way it
        # moves for a person, including any clamping the WebView applies.
        for _ in range(spec.scroll_attempts):
            if t.in_viewport:
                break
            if t.off_screen == "covered":
                # Something is on top of it. The wheel will not move an overlay,
                # so scrolling here would only burn attempts and then blame the
                # scroll bound for what is really a modal in the way.
                break
            self.actuator.send(
                scroll(spec.scroll_step if t.off_screen == "above" else -spec.scroll_step)
            )
            self._sleep(spec.settle_s)
            t = resolve()
        # The pane keeps moving after the wheel event: the WebView animates the
        # scroll. A coordinate read mid-animation is stale by the time the click
        # lands, and the click goes to whatever is at that spot once the pane
        # stops — on 2026-08-05 that was the backdrop, which closed the modal and
        # made the app look like it had refused to uninstall. So require the
        # position to hold still before acting on it.
        for _ in range(spec.settle_attempts):
            self._sleep(spec.settle_s)
            again = resolve()
            if (again.x, again.y) == (t.x, t.y) and again.in_viewport:
                t = again
                break
            t = again
        if not t.in_viewport:
            # Clicking its coordinate anyway would land on whatever *is* at that
            # spot — the backdrop, the neighbouring control — and the run would
            # fail later, somewhere else, wearing a disguise.
            raise RuntimeError(
                f"step {step.name!r}: target {t.text!r} is still {t.off_screen or 'out'} "
                f"of the viewport after {spec.scroll_attempts} scroll attempts "
                "(is the pointer over the scrolling pane?)"
            )
        self.actuator.send(click(t.x, t.y))
        if step.target.type_text is not None:
            self.actuator.send(type_text(step.target.type_text))

    def _run_step(self, step: J.Step) -> StepResult:
        try:
            self._act(step)
        except Exception as e:  # noqa: BLE001 - an actuation failure is a step failure
            return StepResult(step.name, step.trigger, verdict=None, error=repr(e))

        visual: Optional[VisionAssessment] = None
        appeared: Optional[bool] = None
        frame: Optional[Frame] = None

        if step.trigger == J.POLL_UNTIL_VISIBLE and step.vision_question:
            frame, visual, appeared = poll_until_visible(
                self.screen,
                self.assessor,
                step.vision_question,
                timeout=step.poll_timeout,
                now=self._now,
                sleep=self._sleep,
            )
            visual_ok = appeared
        else:
            frame = (
                settle(
                    self.screen,
                    change_probe=self._change_probe,
                    now=self._now,
                    sleep=self._sleep,
                )
                if step.settle
                else self.screen.grab()
            )
            if step.vision_question:
                # ROI narrows only the assessor's copy — `frame` stays full for
                # the forensic and the saved-frame trail.
                assess_frame = (
                    self._crop(frame, step.roi) if step.roi is not None else frame
                )
                visual = self.assessor.assess(assess_frame, step.vision_question)
                visual_ok = visual.visible
            else:
                visual_ok = True  # visual oracle not asked for this step

        # Kernel oracle (the hard cross-check).
        if step.kernel_probe is not None:
            try:
                kernel_ok = bool(step.kernel_probe.check())
            except Exception as e:  # noqa: BLE001
                return StepResult(
                    step.name,
                    step.trigger,
                    verdict=None,
                    visual=visual,
                    appeared=appeared,
                    forensic=frame,
                    error=f"kernel probe: {e!r}",
                )
        else:
            kernel_ok = True  # no kernel cross-check declared

        verdict = cross_check(kernel_ok, visual_ok)
        forensic = None if verdict.ok else frame  # keep the pixels when it broke
        return StepResult(
            name=step.name,
            trigger=step.trigger,
            verdict=verdict,
            visual=visual,
            appeared=appeared,
            forensic=forensic,
        )

    @staticmethod
    def _tier(results: List[StepResult]) -> str:
        hard = any(r.error or (r.verdict and r.verdict.hard_fail) for r in results)
        if hard:
            return "fail"
        soft = any(r.verdict and r.verdict.soft_fail for r in results)
        return "warn" if soft else "pass"
