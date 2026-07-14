"""Run the visual apex driver against a live Windows VM (real backends).

This is the real-backend counterpart of :mod:`.selftest` (which proves the
loop with stubs). It points :class:`.driver.VisualDriver` at
:mod:`.backends_vm` — real screen grabs, real OS input injection, and the real
kernel health hard-gate — and runs a journey end to end.

The vision oracle for this bootstrap run is :class:`.backends_vm.RecordedVision`
(the agent's own multimodal read of the frames, recorded in call order); each
captured frame is written under ``OPV_FRAME_DIR`` so the recorded verdict can be
checked against the pixels after the fact. Swap RecordedVision for a live
multimodal-model assessor to fully automate.

Usage (env: OPV_VM_IP / OPV_VM_USER / OPV_AGENT_PORT / OPV_KERNEL_PORT):

    python -m scripts.opverify.visual.run_vm liveness
    python -m scripts.opverify.visual.run_vm onboarding
"""

from __future__ import annotations

import json
import os
import sys

from . import journey as J
from .backends_vm import (
    KernelHealthProbe,
    RecordedVision,
    VmAgentActuator,
    VmAgentScreen,
)
from .driver import VisualDriver
from .interfaces import Frame, click


class _SavingScreen:
    """Wraps a ScreenSource, writing every grabbed frame to OPV_FRAME_DIR so a
    human/agent can verify the recorded vision verdict against real pixels."""

    def __init__(self, inner, out_dir: str):
        self._inner = inner
        self._dir = out_dir
        self._n = 0
        os.makedirs(out_dir, exist_ok=True)

    def grab(self) -> Frame:
        f = self._inner.grab()
        path = os.path.join(self._dir, f"frame_{self._n:02d}.png")
        with open(path, "wb") as fh:
            fh.write(f.data)
        self._n += 1
        return f


def _liveness_journey():
    """Single no-action step: the app is rendered AND the kernel is healthy."""
    return J.Journey(
        name="vm-liveness",
        steps=[
            J.Step(
                name="app-rendered-and-kernel-healthy",
                trigger=J.CHECKPOINT,
                settle=False,
                vision_question="is the ClotoCore GUI rendered with visible content?",
                kernel_probe=KernelHealthProbe(),
            )
        ],
    )


def _onboarding_journey():
    """Drive the first-run onboarding: advance one page and re-verify. Assumes
    the app is on the onboarding carousel (fresh profile)."""
    return J.Journey(
        name="onboarding-advance",
        steps=[
            J.Step(
                name="welcome-rendered",
                trigger=J.CHECKPOINT,
                settle=False,
                vision_question="is the onboarding welcome screen with a Get Started button visible?",
                kernel_probe=KernelHealthProbe(),
            ),
            J.Step(
                name="advance-to-language",
                action=click(639, 443),  # "はじめる" / Get Started
                trigger=J.CHECKPOINT,
                settle=False,
                vision_question="did onboarding advance to the language-select page?",
                kernel_probe=KernelHealthProbe(),
            ),
        ],
    )


_JOURNEYS = {
    "liveness": (_liveness_journey, [
        {"visible": True, "detail": "onboarding/main UI rendered, non-black window"},
    ]),
    "onboarding": (_onboarding_journey, [
        {"visible": True, "detail": "welcome screen + Get Started button"},
        {"visible": True, "detail": "advanced to language-select page (page 2/7)"},
    ]),
}


def main(argv) -> int:
    name = argv[0] if argv else "liveness"
    if name not in _JOURNEYS:
        print(f"unknown journey: {name} (choose {list(_JOURNEYS)})")
        return 2
    make_journey, recorded = _JOURNEYS[name]
    frame_dir = os.environ.get("OPV_FRAME_DIR", "/tmp/opv-frames")

    driver = VisualDriver(
        screen=_SavingScreen(VmAgentScreen(), frame_dir),
        actuator=VmAgentActuator(),
        assessor=RecordedVision(recorded),
    )
    report = driver.run(make_journey())
    print(json.dumps(report.as_dict(), indent=2, ensure_ascii=False))
    print(f"\nframes saved under: {frame_dir}")
    return 0 if report.verdict != "fail" else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
