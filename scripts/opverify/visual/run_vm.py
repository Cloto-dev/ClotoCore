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
    python -m scripts.opverify.visual.run_vm danger-zone   # ledger is default-on
    python -m scripts.opverify.visual.run_vm liveness --no-ledger  # explicit skip
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time

from .. import ledger as ledger_mod
from . import journey as J
from .backends_vm import (
    KernelApiProbe,
    KernelJsonFetch,
    RecordedVision,
    SshTunnel,
    TunnelActuator,
    TunnelApiProbe,
    TunnelHashSource,
    TunnelHealthProbe,
    TunnelJsonFetch,
    TunnelScreen,
    VmAgentActuator,
    VmAgentHashSource,
    make_cofetch_backend,
)
from .assessor_cache import CachingAssessor
from .driver import VisualDriver
from .interfaces import Frame, click, move, scroll
from .live_assessor import AgentHandshakeAssessor


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


def derive_danger_zone_questions(plan1: dict, plan2: dict) -> tuple:
    """Build the tier-1 / tier-2 visual questions from the kernel's own
    ``/api/system/uninstall/plan`` responses.

    Authoring-time question text baked in the plan's shape ("exactly one
    item", "8 items") and went stale when the plan changed — the assessor's
    lenient reading then let a mismatching frame agree_pass (observed
    2026-07-30). Deriving the expected count / data dir / warning clauses from
    the same endpoint the kernel probe hits makes the visual oracle assert the
    kernel's *current* answer, so a GUI that disagrees with it fails the step
    as FRONTEND_BUG instead of slipping through.
    """
    s1, s2 = plan1["summary"], plan2["summary"]
    n1, n2 = s1["entries"], s2["entries"]
    elev = (
        " and say administrator approval will be requested"
        if s1["needs_elevation"]
        else ""
    )
    secret = (
        ", with a warning that credentials will be destroyed"
        if s2["contains_secret"]
        else ""
    )
    data_dir = plan2["plan"]["data_dir"]
    q1 = (
        f"does the enumeration list exactly {n1} item{'s' if n1 != 1 else ''}, "
        f"including the app executable under Program Files{elev}?"
    )
    q2 = (
        f"did the list re-enumerate to {n2} item{'s' if n2 != 1 else ''} "
        f"including paths under {data_dir}{secret}?"
    )
    return q1, q2


def _fetch_plan(fetch_json, tier: int) -> dict:
    path = f"/api/system/uninstall/plan?tier={tier}"
    body = fetch_json(path)
    # Every kernel handler wraps its payload in a {"data": ...} envelope
    # (handlers/response.rs ok_data) — the 2026-07-31 apex run crashed here
    # because this checked the envelope for the payload's keys.
    body = body.get("data", body)
    if "summary" not in body or "plan" not in body:
        raise RuntimeError(
            f"unexpected plan response from {path} (403? bad OPV_API_KEY?): "
            f"{str(body)[:200]}"
        )
    return body


def _liveness_journey(health_probe, make_api_probe, fetch_json):
    """Single no-action step: the app is rendered AND the kernel is healthy."""
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


def _onboarding_journey(health_probe, make_api_probe, fetch_json):
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


def _agents_journey(health_probe, make_api_probe, fetch_json):
    """Operation-level dual oracle: the GUI is rendered (visual) AND the kernel's
    authenticated /api/agents confirms the seeded default agent exists (op-level
    kernel hard-gate). Requires OPV_API_KEY = the CLOTO_API_KEY the harness
    launched the GUI with."""
    return J.Journey(
        name="agents-seeded",
        steps=[
            J.Step(
                name="default-agent-present",
                trigger=J.CHECKPOINT,
                settle=False,
                vision_question="is the ClotoCore GUI rendered with visible content?",
                kernel_probe=make_api_probe("/api/agents", '"agent_type":"agent"'),
            )
        ],
    )


def _scroll_steps(label: str, amount: int, times: int):
    """`times` wheel events of `amount`, as positioning steps that assert
    nothing. Splitting the travel is not cosmetic: the WebView caps how far one
    wheel event moves a pane, so a single large delta silently stops short."""
    return [
        J.Step(
            name=f"scroll-{label}-{i + 1}",
            action=scroll(amount),
            trigger=J.CHECKPOINT,
            settle=False,
        )
        for i in range(times)
    ]


def _danger_zone_journey(health_probe, make_api_probe, fetch_json):
    """Settings → Health → Danger Zone, dry-run only (`docs/DEFENDER_DESIGN.md`
    §7). The GUI's enumeration is cross-checked against the kernel's own
    `/api/system/uninstall/plan` for the same scope: the user is told, in
    pixels, exactly what a purge would remove, and the kernel is asked the same
    question over the admin API. Widening application-only → +user data must
    move both oracles together.

    Nothing here is destructive: the plan endpoint is read-only, and the journey
    stops short of the admin-key field, so the uninstall button stays disabled.
    Executing a purge is the VM-tier kernel scenario (CSC Task #406), not this.

    Coordinates are for the 1280×800 VM at the app's default zoom. A drifted
    coordinate does not silently pass: the step's visual question fails, because
    the assessor is asked what the frame actually shows.

    The tier-1 / tier-2 questions are derived at construction time from the
    plan endpoint itself (:func:`derive_danger_zone_questions`) — the expected
    counts are the kernel's current answer, never an authored baseline.
    """
    q_tier1, q_tier2 = derive_danger_zone_questions(
        _fetch_plan(fetch_json, 1), _fetch_plan(fetch_json, 2)
    )
    return J.Journey(
        name="danger-zone-dry-run",
        steps=[
            J.Step(
                name="app-rendered",
                trigger=J.CHECKPOINT,
                settle=False,
                vision_question="is the ClotoCore main window rendered with visible content?",
                kernel_probe=health_probe,
            ),
            J.Step(
                name="open-settings",
                action=click(66, 592),  # left rail: 設定 / Settings
                trigger=J.CHECKPOINT,
                vision_question="is the SETTINGS modal open?",
                kernel_probe=health_probe,
            ),
            J.Step(
                name="open-health",
                action=click(259, 339),  # settings nav: ヘルス / Health
                trigger=J.CHECKPOINT,
                vision_question="does the settings pane show a system-health check list?",
                kernel_probe=health_probe,
            ),
            J.Step(
                name="hover-health-pane",
                action=move(720, 450),  # the wheel acts on the pane under the pointer
                trigger=J.CHECKPOINT,
                settle=False,
            ),
            # One wheel event does not scroll an arbitrary distance — the WebView
            # caps how far a single delta moves the pane, so a lone scroll(-3600)
            # stops short of the bottom (measured 2026-07-27). Repeat instead.
            *_scroll_steps("to-danger-zone", -600, 6),
            J.Step(
                name="danger-zone-visible",
                trigger=J.CHECKPOINT,
                vision_question="is a red-bordered DANGER ZONE card visible with a button to review what would be removed?",
                kernel_probe=health_probe,
            ),
            J.Step(
                name="open-the-plan",
                action=click(574, 524),  # "review what would be removed"
                trigger=J.CHECKPOINT,
                vision_question="are cumulative scope checkboxes shown, with the narrowest (application only) checked and disabled?",
                kernel_probe=make_api_probe(
                    "/api/system/uninstall/plan?tier=1", '"tier":"application"'
                ),
            ),
            # The card grew when the plan rendered: ride to the bottom again, then
            # back up one notch so the entry list — not the notes — is on screen.
            *_scroll_steps("plan-into-view", -600, 6),
            J.Step(
                name="tier1-enumeration",
                action=scroll(400),
                trigger=J.CHECKPOINT,
                vision_question=q_tier1,
                kernel_probe=make_api_probe(
                    "/api/system/uninstall/plan?tier=1", '"tier":"application"'
                ),
            ),
            # Ride back up one notch so the scope-checkbox column is fully on
            # screen before clicking. The 2026-07-31 apex run proved why the
            # click needs its own visual verification: the old blind click at
            # (455,227) landed on "+ everything else" (the column had scrolled)
            # and cumulative tiers silently over-selected to 11 items.
            J.Step(
                name="scroll-back-to-scopes",
                action=scroll(400),
                trigger=J.CHECKPOINT,
                settle=False,
            ),
            J.Step(
                name="widen-to-user-data",
                action=click(455, 453),  # "+ ユーザーデータ" checkbox (measured 2026-07-31)
                trigger=J.CHECKPOINT,
                vision_question="is the '+ user data' scope checkbox now checked, while the two wider scopes (large assets / everything else) remain unchecked?",
                kernel_probe=make_api_probe(
                    "/api/system/uninstall/plan?tier=2", '"tier":"user_data"'
                ),
            ),
            J.Step(
                name="tier2-enumeration",
                action=scroll(-400),
                trigger=J.CHECKPOINT,
                vision_question=q_tier2,
                kernel_probe=make_api_probe(
                    "/api/system/uninstall/plan?tier=2", '"tier":"user_data"'
                ),
            ),
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
    "danger-zone": (
        _danger_zone_journey,
        # RecordedVision fallback (OPV_ASSESSOR=recorded). One entry per assessed
        # step, in call order; the two positioning steps ask nothing. Prefer
        # OPV_ASSESSOR=handshake — a pre-recorded verdict cannot notice drift.
        [
            {"visible": True, "detail": "main window rendered"},
            {"visible": True, "detail": "settings modal open"},
            {"visible": True, "detail": "health check list shown"},
            {"visible": True, "detail": "DANGER ZONE card with review button"},
            {"visible": True, "detail": "scope checkboxes, tier 1 checked+disabled"},
            {"visible": True, "detail": "tier-1 enumeration matches the plan count"},
            {"visible": True, "detail": "user-data checkbox checked, wider tiers unchecked"},
            {"visible": True, "detail": "tier-2 re-enumeration matches the plan count"},
        ],
    ),
}


# The OS label recorded on an apex ledger row. It names the machine *under
# verification*, not the orchestrating host: the apex drives the real installed
# GUI on the Windows VM (VM 104 — see VM_EXECUTOR_RUNBOOK.md), while the
# orchestrator typically runs on macOS. Hardcoded because the apex has exactly
# one VM target today; a second one would make this a CLI argument.
APEX_OS_LABEL = "windows-vm"


# Settle hash-poll (agent /grabhash) is OPT-IN via OPV_SETTLE_HASHPOLL=1.
# Measured net-negative at current per-call costs (#234) — off by default.
def _hashpoll_enabled() -> bool:
    return bool(os.environ.get("OPV_SETTLE_HASHPOLL"))


def _build_transport(transport: str):
    """Return (screen, actuator, health_probe, make_api_probe, fetch_json,
    change_probe, teardown) for the chosen transport.

    - ``tunnel`` (default, #235): a persistent SSH port-forward — every call is a
      plain local HTTP hit, no per-call ssh/PowerShell/curl spawn (measured ~2.3x
      on /grab, kernel probes ~350→7ms). One master for the whole run.
    - ``curl``: the ssh + ``curl.exe`` transport with the grab+liveness co-fetch
      fusion (#233). Kept as a no-setup fallback."""
    if transport == "curl":
        screen, health_probe = make_cofetch_backend()
        change_probe = VmAgentHashSource().hash if _hashpoll_enabled() else None
        return (
            screen,
            VmAgentActuator(),
            health_probe,
            KernelApiProbe,
            KernelJsonFetch(),
            change_probe,
            lambda: None,
        )

    tunnel = SshTunnel().open()
    change_probe = TunnelHashSource(tunnel).hash if _hashpoll_enabled() else None
    return (
        TunnelScreen(tunnel),
        TunnelActuator(tunnel),
        TunnelHealthProbe(tunnel),
        lambda path, want: TunnelApiProbe(tunnel, path, want),
        TunnelJsonFetch(tunnel),
        change_probe,
        tunnel.close,
    )


def _parse_args(argv):
    p = argparse.ArgumentParser(
        prog="opverify-apex", description=__doc__.splitlines()[0]
    )
    p.add_argument(
        "journey",
        nargs="?",
        default="liveness",
        choices=sorted(_JOURNEYS),
        help="journey to drive (default: liveness)",
    )
    p.add_argument(
        "--ledger",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="append this apex run to qa/opverify/history.jsonl and check for "
        "regressions vs the prior apex baseline (apex rows are compared only "
        "against apex rows). Default ON — an unrecorded run is invisible to "
        "the usage ledger, so skipping (--no-ledger) must be a deliberate, "
        "visible choice, not a forgotten flag",
    )
    p.add_argument(
        "--history",
        default=None,
        help="override ledger history path (default qa/opverify/history.jsonl)",
    )
    return p.parse_args(argv)


def main(argv) -> int:
    args = _parse_args(argv)
    name = args.journey
    make_journey, recorded = _JOURNEYS[name]
    frame_dir = os.environ.get("OPV_FRAME_DIR", "/tmp/opv-frames")
    transport = os.environ.get("OPV_TRANSPORT", "tunnel")

    # Assessor: 'recorded' (bootstrap — replays pre-recorded verdicts in call
    # order) or 'handshake' (live — a Sonnet VM-executor subagent reads each
    # frame and writes the verdict; unattended AI, no API key). #237.
    assessor_kind = os.environ.get("OPV_ASSESSOR", "recorded")
    handshake = None
    if assessor_kind == "handshake":
        handshake = AgentHandshakeAssessor(
            os.environ.get("OPV_EXCHANGE_DIR", "/tmp/opv-exchange")
        )
        # Cache verdicts by (frame, question) so a poll-heavy checkpoint doesn't
        # re-ask the live assessor about a byte-identical frame (#236). Safe only
        # for the stateless live assessor — never wrap RecordedVision.
        assessor = CachingAssessor(handshake)
    else:
        assessor = RecordedVision(recorded)

    screen, actuator, health_probe, make_api_probe, fetch_json, change_probe, teardown = (
        _build_transport(transport)
    )
    try:
        driver = VisualDriver(
            screen=_SavingScreen(screen, frame_dir),
            actuator=actuator,
            assessor=assessor,
            change_probe=change_probe,
        )
        report = driver.run(make_journey(health_probe, make_api_probe, fetch_json))
    finally:
        teardown()
        if handshake is not None:
            handshake.signal_done()
    print(json.dumps(report.as_dict(), indent=2, ensure_ascii=False))
    print(f"\nframes saved under: {frame_dir}  (transport={transport})")

    regressed = False
    if args.ledger:
        entry, regressions = ledger_mod.record_apex(
            report.as_dict(),
            ts=time.time(),
            os_label=APEX_OS_LABEL,
            history_path=args.history,
        )
        print(
            f"\nledger: recorded {entry.run_id} "
            f"(git={entry.git_sha or 'n/a'}, os={entry.os}, "
            f"steps={entry.ops_passed}/{entry.ops_total})"
        )
        for reg in regressions:
            regressed = True
            print(
                f"  !! REGRESSION [{reg.kind}] {reg.detail} (vs {reg.baseline_run_id})"
            )

    return 0 if report.verdict != "fail" and not regressed else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
