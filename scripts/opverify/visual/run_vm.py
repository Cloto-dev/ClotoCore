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
    KernelApiProbe,
    RecordedVision,
    VmAgentActuator,
    VmAgentHashSource,
    make_cofetch_backend,
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


def _liveness_journey(health_probe):
    """Single no-action step: the app is rendered AND the kernel is healthy.
    The liveness gate is co-fetched with the grab (one round trip)."""
    return J.Journey(
        name="vm-liveness",
        steps=[
            J.Step(
                name="app-rendered-and-kernel-healthy",
                trigger=J.CHECKPOINT,
                settle=False,
                vision_question="is the ClotoCore GUI rendered with visible content?",
                kernel_probe=health_probe,
            )
        ],
    )


def _onboarding_journey(health_probe):
    """Drive the first-run onboarding: advance one page and re-verify. Assumes
    the app is on the onboarding carousel (fresh profile). Each health gate is
    co-fetched with that step's grab (one round trip per checkpoint)."""
    return J.Journey(
        name="onboarding-advance",
        steps=[
            J.Step(
                name="welcome-rendered",
                trigger=J.CHECKPOINT,
                settle=False,
                vision_question="is the onboarding welcome screen with a Get Started button visible?",
                kernel_probe=health_probe,
            ),
            J.Step(
                name="advance-to-language",
                action=click(639, 443),  # "はじめる" / Get Started
                trigger=J.CHECKPOINT,
                settle=False,
                vision_question="did onboarding advance to the language-select page?",
                kernel_probe=health_probe,
            ),
        ],
    )


def _agents_journey(health_probe):
    """Operation-level dual oracle: the GUI is rendered (visual) AND the kernel's
    authenticated /api/agents confirms the seeded default agent exists (op-level
    kernel hard-gate). Requires OPV_API_KEY = the CLOTO_API_KEY the harness
    launched the GUI with. The op-level gate is authenticated, so it stays a
    separate probe (not co-fetchable with the unauthenticated liveness); the grab
    still co-fetches health harmlessly on the same round trip."""
    return J.Journey(
        name="agents-seeded",
        steps=[
            J.Step(
                name="default-agent-present",
                trigger=J.CHECKPOINT,
                settle=False,
                vision_question="is the ClotoCore GUI rendered with visible content?",
                kernel_probe=KernelApiProbe("/api/agents", '"agent_type":"agent"'),
            )
        ],
    )


_JOURNEYS = {
    "liveness": (
        _liveness_journey,
        [
            {
                "visible": True,
                "detail": "onboarding/main UI rendered, non-black window",
            },
        ],
    ),
    "onboarding": (
        _onboarding_journey,
        [
            {"visible": True, "detail": "welcome screen + Get Started button"},
            {"visible": True, "detail": "advanced to language-select page (page 2/7)"},
        ],
    ),
    "agents": (
        _agents_journey,
        [
            {"visible": True, "detail": "ClotoCore UI rendered (onboarding/main)"},
        ],
    ),
}


def main(argv) -> int:
    name = argv[0] if argv else "liveness"
    if name not in _JOURNEYS:
        print(f"unknown journey: {name} (choose {list(_JOURNEYS)})")
        return 2
    make_journey, recorded = _JOURNEYS[name]
    frame_dir = os.environ.get("OPV_FRAME_DIR", "/tmp/opv-frames")

    # Fused backend: grab + liveness health share one ssh round trip.
    screen, health_probe = make_cofetch_backend()
    # Settle hash-poll (agent /grabhash) is OPT-IN via OPV_SETTLE_HASHPOLL=1.
    # Measured on VM104 (2026-07-15): /grabhash saves only ~23ms/poll (~4%) over
    # /grab — the per-call cost is dominated by ssh+PowerShell+screen-capture
    # overhead, not the PNG encode/transfer hash-polling skips. With the extra
    # final grab, hash-polling only beats grab-based settle past ~27 polls, so it
    # is a net loss for realistic settles (2–8 polls) today. Keep the primitive
    # wired but off; revisit once #235 (persistent channel) cuts per-call
    # overhead and makes the PNG fraction worth skipping.
    change_probe = (
        VmAgentHashSource().hash if os.environ.get("OPV_SETTLE_HASHPOLL") else None
    )
    driver = VisualDriver(
        screen=_SavingScreen(screen, frame_dir),
        actuator=VmAgentActuator(),
        assessor=RecordedVision(recorded),
        change_probe=change_probe,
    )
    report = driver.run(make_journey(health_probe))
    print(json.dumps(report.as_dict(), indent=2, ensure_ascii=False))
    print(f"\nframes saved under: {frame_dir}")
    return 0 if report.verdict != "fail" else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
