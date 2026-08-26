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
import secrets
import sys
import time
from dataclasses import dataclass
from typing import Optional

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
from . import affordance_coverage as AC
from . import fixtures as FX
from . import os_oracle as OS
from .assessor_cache import CachingAssessor
from .cdp import CdpTargeter, CdpTunnel, captured_size, space_mismatch
from .driver import VisualDriver
from .interfaces import Frame, click, move, press_key, scroll, type_text
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


# Chat view geometry on the 1280x800 VM screen (measured 2026-08-03 against
# 0.6.8-beta.3). The composer runs along the bottom; clicking mid-width focuses
# it without hitting the attach / mic / model-picker controls on the left or the
# send button on the right.
_CHAT_INPUT_XY = (800, 714)


class _AwaitingProbe:
    """A kernel probe for an *asynchronous* outcome: retry until true or the
    bound elapses.

    The driver evaluates ``kernel_probe`` exactly once, after the visual poll
    returns. That is right for a synchronous step, but for a step waiting on an
    LLM round trip it ties the kernel sample to how long the *visual* oracle
    happened to take — so under ``OPV_ASSESSOR=recorded``, whose canned verdict
    returns instantly, the kernel is sampled before the reply can possibly
    exist and the step fails with ``backend_or_hidden`` on a perfectly healthy
    app (observed 2026-08-03). Letting the kernel wait on its own clock
    decouples the two oracles, so the recorded fallback still carries a real
    kernel half.
    """

    def __init__(self, inner, timeout: float = 60.0, interval: float = 2.0):
        self._inner = inner
        self._timeout = timeout
        self._interval = interval

    def check(self) -> bool:
        deadline = time.time() + self._timeout
        while True:
            if self._inner.check():
                return True
            if time.time() >= deadline:
                return False
            time.sleep(self._interval)


def _chat_nonce() -> str:
    """A per-run token the reply must echo. Alphanumeric on purpose: the
    session-1 agent types via pyautogui ``write()``, which mis-maps some shifted
    characters on the VM's JP keyboard layout (VM_EXECUTOR_RUNBOOK.md, "Known
    harness artifacts") — an alphanumeric nonce is unaffected."""
    return "opv" + secrets.token_hex(4)


def _chat_journey(health_probe, make_api_probe, fetch_json):
    """The headline dual-oracle journey: a real user types into the chat box,
    the assistant's reply **renders**, and the kernel **persists** it.

    Driven ad hoc on 2026-07-14 (FIRST_RUN.md) but never registered, so it could
    not be re-run — which is exactly what the opverify anti-rot design is meant
    to prevent. Registered here.

    The kernel oracle is deliberately narrow. The nonce also appears in the
    user's own message echoed back in ``/api/history``, so matching the nonce
    alone would pass even if the assistant never answered. Requiring
    ``"content":"<nonce>","engine_id":"`` pins the match to a ThoughtResponse
    whose content is *exactly* the nonce — the engine name is left unpinned so
    the journey survives a default-engine change.

    Preconditions: the app is on an agent's chat view (not onboarding, not the
    settings modal) and a reasoning engine is connected. A disconnected engine
    renders an error bubble instead of a reply, which surfaces as AGREE_FAIL
    rather than a silent pass.
    """
    nonce = _chat_nonce()
    return J.Journey(
        name="chat-render",
        steps=[
            J.Step(
                name="chat-view-rendered",
                trigger=J.CHECKPOINT,
                settle=False,
                vision_question=(
                    "is a chat view rendered, with a message input box along "
                    "the bottom of the window?"
                ),
                kernel_probe=health_probe,
            ),
            # Positioning steps: they assert nothing, so they ask the assessor
            # nothing (image tokens are the dominant cost).
            J.Step(
                name="focus-chat-input",
                action=click(*_CHAT_INPUT_XY),
                trigger=J.CHECKPOINT,
                settle=False,
            ),
            J.Step(
                name="type-nonce",
                action=type_text(
                    f"Reply with exactly this token and nothing else: {nonce}"
                ),
                trigger=J.CHECKPOINT,
                settle=False,
            ),
            J.Step(
                name="send",
                action=press_key("enter"),
                trigger=J.CHECKPOINT,
                settle=False,
            ),
            # Wait for the backend first, on its own clock. Splitting the wait
            # out of the visual step is deliberate: the driver samples
            # kernel_probe once, after the visual poll returns, so a combined
            # step would sample the kernel at whatever moment the *visual*
            # oracle happened to finish — instantly, under the canned recorded
            # assessor, which is before any reply can exist.
            J.Step(
                name="await-reply-persisted",
                trigger=J.CHECKPOINT,
                settle=False,
                kernel_probe=_AwaitingProbe(
                    make_api_probe(
                        "/api/history", f'"content":"{nonce}","engine_id":"'
                    ),
                    timeout=60.0,
                ),
            ),
            # Asserted WITHOUT scrolling first, deliberately. Until bug-498 the
            # thread did not follow its own tail — the turn rendered below the
            # fold — and this journey scrolled before asserting so the step
            # answered "did the reply render" rather than "did the pane follow".
            # With the fix landed the two questions collapse into one: a reply
            # the user can see without touching the wheel. The scroll steps are
            # gone rather than kept "just in case", because a journey that
            # scrolls first can never fail on a regression of the fix.
            J.Step(
                name="reply-rendered-without-scrolling",
                trigger=J.CHECKPOINT,
                settle=False,
                vision_question=(
                    f"is an assistant reply — a bubble on the LEFT, not the "
                    f"user's own message on the right — whose text is "
                    f"'{nonce}' visible in this frame, without anyone having "
                    f"scrolled the transcript?"
                ),
            ),
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
    Executing a purge is the VM-tier kernel scenario, not this.

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
                # The enumeration lives in the settings content pane; a count
                # question doesn't need the rail, taskbar, or backdrop (#446).
                roi=(420, 155, 600, 475),
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
                roi=(420, 330, 600, 300),  # the scope-checkbox column
            ),
            J.Step(
                name="tier2-enumeration",
                action=scroll(-400),
                trigger=J.CHECKPOINT,
                vision_question=q_tier2,
                kernel_probe=make_api_probe(
                    "/api/system/uninstall/plan?tier=2", '"tier":"user_data"'
                ),
                roi=(420, 155, 600, 475),
            ),
        ],
    )


def _danger_zone_purge_journey(health_probe, make_api_probe, fetch_json):
    """The danger zone driven to its **outcome**: execute, the app ends, the
    detached helper purges, nothing of the product is left on the machine.

    Why this exists as its own journey rather than a longer `danger-zone`: that
    one is a dry run by construction — it never presses the button — and
    bug-499 (the full uninstall removed nothing and hung on an overlay) lived
    behind exactly that boundary while 22/22 steps agreed pass. A preview
    journey cannot fail on an outcome it never reaches.

    Three things make it different from every journey before it:

    * **It runs at tier 4** (everything). The narrower tiers deliberately leave
      the ARP entry and the vendor key behind, so "residue is zero" is only a
      meaningful assertion at the widest scope.
    * **Its oracles outlive the kernel.** After the confirm, the HTTP oracle is
      gone on purpose; what must be true is about the machine, and
      :mod:`.os_oracle` asserts it.
    * **The sweep runs before as well as after.** A detector that has only ever
      reported zero has not been shown to work — the lesson bug-497 left.

    Destructive, and only sane on a VM that can be put back. It needs an install
    with data to remove: `PurgeReportClean` fails a run whose report is all
    `absent`, because a purge with nothing to purge verifies nothing.
    """
    plan = _fetch_plan(fetch_json, 4)
    data_dir = plan["plan"]["data_dir"]
    entries = plan["summary"]["entries"]
    if entries == 0:
        raise RuntimeError(
            "the tier-4 plan is empty — this journey needs an installed app with "
            "data to remove. Roll the VM to a fixture that has some; "
            "passing on an empty plan would verify nothing."
        )
    admin_key = os.environ.get("OPV_API_KEY", "")
    if not admin_key:
        raise RuntimeError("OPV_API_KEY is required: gate 3 asks for the admin key")

    # Locale-independent anchors: the VM runs the Japanese pack, the locale
    # files are authored in English, and buttons render through
    # `text-transform: uppercase`. Matching is case-insensitive.
    SETTINGS = ("設定", "settings")
    HEALTH = ("ヘルス", "health")
    REVIEW = ("削除される対象を確認", "review what would be removed")
    EVERYTHING = ("その他すべて", "everything else")
    KEY_FIELD = ("管理 API キー", "admin api key")
    # The card's button and the confirm dialog's button are on screen together
    # once the dialog opens, and one name contains the other. The card's is
    # matched by the part the dialog's lacks; the dialog's is matched exactly.
    EXECUTE = ("をアンインストール", "uninstall clotocore")
    CONFIRM = ("アンインストール", "uninstall")

    return J.Journey(
        name="danger-zone-purge",
        steps=[
            # Step one is also the fixture check. "Is the app rendered?" is too
            # loose to be that: a freshly installed app renders its onboarding
            # carousel and passes, and then eight targets in a row fail to
            # resolve against a screen the journey was never written for
            # (measured 2026-08-05 — after a purge removed the data directory,
            # the reinstall came back onboarding and the run read as nine
            # defects). The precondition has to fail here, once, by name.
            J.Step(
                name="app-rendered",
                trigger=J.CHECKPOINT,
                settle=False,
                vision_question=(
                    "is the ClotoCore MAIN window shown — a left navigation rail with "
                    "entries for agents, MCP, CRON and settings — and NOT a first-run "
                    "onboarding or setup screen?"
                ),
                kernel_probe=health_probe,
            ),
            # The detector, shown working on this machine before its zero is
            # used as evidence at the end.
            J.Step(
                name="residue-present-before-purge",
                trigger=J.CHECKPOINT,
                settle=False,
                kernel_probe=OS.ResidueSweep(data_dir=data_dir, expect="present"),
            ),
            J.Step(
                name="open-settings",
                target=J.TargetSpec(contains=SETTINGS, nth=0),
                trigger=J.CHECKPOINT,
                vision_question="is the SETTINGS modal open?",
                kernel_probe=health_probe,
            ),
            J.Step(
                name="open-health",
                target=J.TargetSpec(contains=HEALTH),
                trigger=J.CHECKPOINT,
                vision_question="does the settings pane show a system-health check list?",
                kernel_probe=health_probe,
            ),
            # The wheel acts under the pointer, and every target below scrolls
            # itself into view from here.
            J.Step(
                name="hover-health-pane",
                action=move(720, 450),
                trigger=J.CHECKPOINT,
                settle=False,
            ),
            J.Step(
                name="open-the-plan",
                target=J.TargetSpec(contains=REVIEW),
                trigger=J.CHECKPOINT,
                # Asked about what opening the card actually puts on screen. The
                # scope checkboxes are below the fold at this moment, so asking
                # about them here fails on the journey's own scroll position
                # rather than on the app (measured 2026-08-05); the next step
                # scrolls to them and asserts them where they are visible.
                vision_question="did the danger-zone card expand into a review of what would be removed, with a scope section?",
                kernel_probe=make_api_probe(
                    "/api/system/uninstall/plan?tier=1", '"tier":"application"'
                ),
            ),
            J.Step(
                name="widen-to-everything",
                target=J.TargetSpec(contains=EVERYTHING),
                trigger=J.CHECKPOINT,
                # About the one checkbox this step acted on, not all four: the
                # column is taller than the space the scroll leaves for it, so
                # "are all four checked" is a question about the scroll position
                # (measured 2026-08-05 — the assessor could see three). That the
                # scope really widened is the kernel probe's job, and it says so
                # authoritatively.
                vision_question="is the widest scope checkbox — the one labelled 'everything else' / 'その他すべて' — checked?",
                kernel_probe=make_api_probe(
                    "/api/system/uninstall/plan?tier=4", '"tier":"everything"'
                ),
            ),
            J.Step(
                name="enter-the-admin-key",
                target=J.TargetSpec(contains=KEY_FIELD, type_text=admin_key),
                trigger=J.CHECKPOINT,
                # Asked about the *state*, never the value: the field masks its
                # contents and the frame is kept as a forensic. Only about the
                # field — the button sits further down the card and asking about
                # both makes the answer depend on the scroll position rather than
                # on the app (measured 2026-08-05).
                vision_question="is the admin-key field filled, showing a masked value rather than empty placeholder text?",
                kernel_probe=make_api_probe(
                    "/api/system/uninstall/plan?tier=4", '"tier":"everything"'
                ),
            ),
            J.Step(
                name="press-uninstall",
                target=J.TargetSpec(contains=EXECUTE, require_enabled=True),
                trigger=J.CHECKPOINT,
                # The last screen before the point of no return states the scope
                # it is about to act on, so this is the last place a wrong scope
                # can be caught — and it is a real risk: the 2026-08-05 run
                # reached this dialog still saying tier 1 because the widening
                # never landed. The expected numbers come from the kernel's own
                # plan, not from an authored baseline.
                vision_question=(
                    f"does the confirmation dialog say the scope is tier 4 "
                    f"and list {entries} items?"
                ),
                kernel_probe=health_probe,
            ),
            # Past this step the kernel is on its way out; no HTTP oracle below.
            J.Step(
                name="confirm-the-uninstall",
                target=J.TargetSpec(contains=CONFIRM, exact=True, require_enabled=True),
                trigger=J.CHECKPOINT,
                settle=False,
            ),
            J.Step(
                name="the-app-ends",
                trigger=J.POLL_UNTIL_VISIBLE,
                poll_timeout=60.0,
                # bug-499's user-visible signature was the opposite of this: a
                # window parked on a shutdown overlay that never resolved.
                vision_question=(
                    "is the ClotoCore window gone from the desktop — no window and "
                    "no shutdown overlay left on screen?"
                ),
                kernel_probe=OS.ProcessAbsent(wait_s=60.0),
            ),
            J.Step(
                name="the-helper-reports",
                trigger=J.CHECKPOINT,
                settle=False,
                kernel_probe=OS.PurgeReportClean(wait_s=180.0),
            ),
            J.Step(
                name="residue-sweep-is-zero",
                trigger=J.CHECKPOINT,
                settle=False,
                kernel_probe=OS.ResidueSweep(data_dir=data_dir, expect="empty"),
            ),
        ],
    )


DEFAULT_CENSUS_REL = os.path.join("qa", "opverify", "affordance-census.json")


def _affordance_coverage(journey) -> Optional[dict]:
    """What share of the app's affordances this journey declares it acts on.

    Returns None when no census has been taken, which the ledger records as
    0 of 0 = *not measured* — deliberately distinct from "covers nothing", so a
    run on a machine without the census cannot look like a coverage collapse.
    """
    root = os.path.dirname(
        os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    )
    path = os.environ.get("OPV_CENSUS", os.path.join(root, DEFAULT_CENSUS_REL))
    if not os.path.exists(path):
        return None
    try:
        with open(path, encoding="utf-8") as fh:
            census = AC.Census.from_json(fh.read())
        report = AC.coverage(census, AC.declared_targets(journey))
    except Exception as e:  # noqa: BLE001 — a missing denominator must not fail a run
        print(f"affordance census unusable ({e}); recording no coverage", file=sys.stderr)
        return None
    return {
        "coverage_pct": report.coverage_pct,
        "covered": report.covered,
        "total": report.total,
        "unmatched": [f"{d.step}: {d.alternatives}" for d in report.unmatched_declarations],
    }


@dataclass
class _Spec:
    """A committed journey: how to build it, what to replay under the
    `recorded` assessor, and the start state it is written against.

    The fixture is declared here rather than inside the factory because it has
    to be known *before* the kernel is reachable — `--rollback-to-fixture` runs
    while the VM is still being put back, and two of the factories read the
    kernel to build their questions.
    """

    factory: object
    recorded: list
    fixture: Optional[str] = None


_JOURNEYS = {
    "liveness": _Spec(
        _liveness_journey,
        [
            {
                "visible": True,
                "detail": "onboarding/main UI rendered, non-black window",
            },
        ],
        # Deliberately none: "did anything render at all" is the one question
        # that is worth asking of whatever state the machine happens to be in.
        fixture=None,
    ),
    "onboarding": _Spec(
        _onboarding_journey,
        [
            {"visible": True, "detail": "welcome screen + Get Started button"},
            {"visible": True, "detail": "advanced to language-select page (page 2/7)"},
        ],
        fixture="first-run",
    ),
    "agents": _Spec(
        _agents_journey,
        [
            {"visible": True, "detail": "ClotoCore UI rendered (onboarding/main)"},
        ],
        fixture="onboarded",
    ),
    "chat-render": _Spec(
        _chat_journey,
        # RecordedVision fallback: one entry per *assessed* step, in call order.
        # The typing / send / wait steps ask nothing. Prefer
        # OPV_ASSESSOR=handshake — a canned "the reply rendered" is worth
        # nothing on the one journey whose whole point is that the reply
        # rendered, unscrolled, where the user is already looking.
        [
            {"visible": True, "detail": "chat view with a bottom input box"},
            {"visible": True, "detail": "assistant reply bubble echoing the nonce"},
        ],
        # Needs an engine, a key AND a transcript taller than the pane: on a
        # short thread every new turn is visible however the scroll logic
        # behaves, so `reply-rendered-without-scrolling` passes on a pane that
        # is broken (the bug-498 class).
        fixture="configured-chat",
    ),
    "danger-zone": _Spec(
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
        fixture="onboarded",
    ),
    "danger-zone-purge": _Spec(
        _danger_zone_purge_journey,
        # Deliberately empty: this journey refuses to run under the `recorded`
        # assessor (see DESTRUCTIVE_JOURNEYS), so there is nothing to replay.
        [],
        fixture="onboarded",
    ),
}


# Journeys that change the machine irreversibly. Two rules apply to them:
# canned visual verdicts are refused (replaying "yes, it rendered" while
# actually uninstalling would be evidence of nothing), and the operator is
# expected to have a way to put the VM back.
DESTRUCTIVE_JOURNEYS = {"danger-zone-purge"}


# The OS label recorded on an apex ledger row. It names the machine *under
# verification*, not the orchestrating host: the apex drives the real installed
# GUI on the Windows guest (see VM_EXECUTOR_RUNBOOK.md), while the
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
    p.add_argument(
        "--check-fixture",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="verify the journey's declared start state before driving it, and "
        "refuse to run when it is not satisfied. Default ON: an unmet "
        "precondition does not fail a journey, it makes it vacuous — an empty "
        "thread or a data-free install produces a green run that asserted "
        "nothing (measured 2026-08-03 / 2026-08-05)",
    )
    p.add_argument(
        "--rollback-to-fixture",
        action="store_true",
        help="roll the VM back to the snapshot of the journey's fixture before "
        "running. DESTRUCTIVE: qm rollback discards whatever is on the VM now, "
        "and there is no snapshot of 'now' to return to — on 2026-08-03 it "
        "threw away the only configured environment in existence. Never "
        "implied by --check-fixture",
    )
    return p.parse_args(argv)


def main(argv) -> int:
    args = _parse_args(argv)
    name = args.journey
    spec = _JOURNEYS[name]
    make_journey, recorded = spec.factory, spec.recorded
    frame_dir = os.environ.get("OPV_FRAME_DIR", "/tmp/opv-frames")
    transport = os.environ.get("OPV_TRANSPORT", "tunnel")

    # Assessor: 'recorded' (bootstrap — replays pre-recorded verdicts in call
    # order) or 'handshake' (live — a Sonnet VM-executor subagent reads each
    # frame and writes the verdict; unattended AI, no API key). #237.
    assessor_kind = os.environ.get("OPV_ASSESSOR", "recorded")
    if name in DESTRUCTIVE_JOURNEYS and assessor_kind != "handshake":
        print(
            f"{name} changes the machine irreversibly; it requires a live visual "
            "oracle (OPV_ASSESSOR=handshake). Replayed verdicts would agree with "
            "anything while the uninstall proceeded.",
            file=sys.stderr,
        )
        return 2
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

    # Putting the VM back happens before any transport is opened: the rollback
    # restarts the guest, which would drop an SSH master opened first.
    if args.rollback_to_fixture:
        if not spec.fixture:
            print(
                f"{name} declares no fixture, so there is nothing to roll back to.",
                file=sys.stderr,
            )
            return 2
        print(
            f"rolling VM {FX.vm_id()} back to fixture {spec.fixture!r} — this "
            f"DISCARDS the machine's current state, which no snapshot holds."
        )
        snapshot = FX.rollback(spec.fixture)
        print(f"  restored {snapshot!r}; session-1 actuator is answering")

    screen, actuator, health_probe, make_api_probe, fetch_json, change_probe, teardown = (
        _build_transport(transport)
    )
    cdp_tunnel = None
    try:
        # The start state, before the journey is even built: two of the
        # factories read the kernel to derive their questions, so a bad state
        # would otherwise surface as a confusing failure inside construction.
        if spec.fixture and args.check_fixture:
            fx_report = FX.verify(spec.fixture, fetch_json)
            for check_name, ok, detail in fx_report.results:
                print(f"fixture[{spec.fixture}] {'ok  ' if ok else 'FAIL'} {check_name}: {detail}")
            if fx_report.error:
                # Could not ask ≠ the answer is no. Exit 4, not 3, and never
                # the rollback remedy: the machine may well be in the wanted
                # state, and rolling it back would discard it (bug-500).
                sys.stdout.flush()
                print(
                    f"\n{name} could not confirm its start state — the kernel did "
                    "not answer the question. NOT running, and NOT concluding the "
                    "VM is in the wrong state.\n",
                    file=sys.stderr,
                )
                print(FX.probe_remedy(fx_report.error), file=sys.stderr)
                return 4
            if not fx_report.ok:
                # The per-check lines above are the evidence for the refusal
                # below; unflushed they arrive after it and read as an answer
                # to a question nobody asked yet.
                sys.stdout.flush()
                print(
                    f"\n{name} needs a start state this machine is not in. Running "
                    "anyway would not fail the journey — it would make its "
                    "assertions vacuous.\n",
                    file=sys.stderr,
                )
                print(FX.remedy(spec.fixture), file=sys.stderr)
                return 3

        journey = make_journey(health_probe, make_api_probe, fetch_json)
        journey.fixture = spec.fixture
        # Targets are resolved live, so the debug port only has to be open for
        # journeys that declare any — the rest keep working with it closed.
        targeter = None
        if any(s.target is not None for s in journey.steps):
            cdp_tunnel = CdpTunnel().open()
            targeter = CdpTargeter(cdp_tunnel)
            # Before the first aim, not after the run reads oddly: check that the
            # frames and the coordinates are in one pixel space (bug-503/504).
            targeter.affordances()
            problem = space_mismatch(targeter.last_frame, captured_size(screen))
            if problem:
                print(f"opverify apex: {problem}", file=sys.stderr)
                return 3
        driver = VisualDriver(
            screen=_SavingScreen(screen, frame_dir),
            actuator=actuator,
            assessor=assessor,
            change_probe=change_probe,
            targeter=targeter,
        )
        report = driver.run(journey)
    finally:
        if cdp_tunnel is not None:
            cdp_tunnel.close()
        teardown()
        if handshake is not None:
            handshake.signal_done()
    print(json.dumps(report.as_dict(), indent=2, ensure_ascii=False))
    print(f"\nframes saved under: {frame_dir}  (transport={transport})")

    regressed = False
    if args.ledger:
        coverage = _affordance_coverage(journey)
        if coverage:
            print(
                f"affordances: {journey.name} declares {coverage['covered']} of "
                f"{coverage['total']} ({coverage['coverage_pct']}%)"
            )
            for miss in coverage["unmatched"]:
                # A declaration matching nothing means this journey has gone
                # stale against the UI, or the census never visited the surface
                # it acts on. Silence here reads as low coverage instead.
                print(f"  unmatched declaration: {miss}", file=sys.stderr)
        entry, regressions = ledger_mod.record_apex(
            report.as_dict(),
            ts=time.time(),
            os_label=APEX_OS_LABEL,
            history_path=args.history,
            assessor=assessor_kind,
            coverage=coverage,
        )
        print(
            f"\nledger: recorded {entry.run_id} "
            f"(git={entry.git_sha or 'n/a'}, os={entry.os}, "
            f"assessor={entry.assessor}, "
            f"steps={entry.ops_passed}/{entry.ops_total})"
        )
        if entry.assessor == "recorded":
            # Say it where the operator is already looking. A recorded row
            # carries no visual evidence — the verdicts were canned before the
            # run — and reading it as an apex pass is the mistake this label
            # exists to prevent.
            print(
                "  note: visual verdicts were REPLAYED, not assessed. "
                "Re-run with OPV_ASSESSOR=handshake for a live visual oracle."
            )
        for reg in regressions:
            regressed = True
            print(
                f"  !! REGRESSION [{reg.kind}] {reg.detail} (vs {reg.baseline_run_id})"
            )

    return 0 if report.verdict != "fail" and not regressed else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
