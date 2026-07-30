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
        crop: Callable[[Frame, tuple], Frame] = crop_frame,
        now: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], None] = time.sleep,
    ):
        self.screen = screen
        self.actuator = actuator
        self.assessor = assessor
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
    def _run_step(self, step: J.Step) -> StepResult:
        try:
            if step.action.kind != "noop":
                self.actuator.send(step.action)
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
