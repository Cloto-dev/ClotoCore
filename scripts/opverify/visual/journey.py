"""Declarative user journeys.

A journey is the golden path a real user walks — for the first prototype, the
chat round-trip: focus the input, type a message, send it, and watch the reply
render. Each step declares *how* it is triggered and *what* both oracles should
find, so the driver stays generic and journeys are data.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import List, Optional, Tuple

from .interfaces import Action, KernelProbe, noop


# Trigger kinds (see module docstring of :mod:`.driver`).
CHECKPOINT = "checkpoint"  # settle, then one capture + assess (backbone)
POLL_UNTIL_VISIBLE = "poll_until_visible"  # async outcome; poll until seen / timeout
KERNEL_EVENT = "kernel_event"  # a kernel event fired → confirm the GUI reflects it


@dataclass
class TargetSpec:
    """*What* to act on, instead of *where* it is.

    A pixel coordinate is only true for the scroll position, window size and
    list length it was measured in; when it drifts the click still lands
    somewhere and the run fails later wearing a disguise (observed twice —
    2026-07-31, 2026-08-05). A step that declares a target has it resolved at
    the moment it acts, through the driver's targeter.

    `contains` matches visible text (a tuple = alternatives, matched
    case-insensitively, so a journey is not pinned to one locale). `type_text`
    is typed after the click — for a secret it is passed by value and never
    logged; the step name is what appears in the report.
    """

    contains: object  # str | tuple[str, ...]
    nth: int = 0
    require_enabled: bool = False
    # Match the whole visible text rather than a substring. Needed when one
    # control's name is a prefix of another's and both are on screen at once.
    exact: bool = False
    type_text: Optional[str] = None
    # Wheel toward the target and re-resolve until it is on screen. The pointer
    # must already be over the pane that scrolls (a `move` step) — the wheel acts
    # where the cursor is. 0 disables it: the target must already be in view.
    scroll_attempts: int = 12
    scroll_step: int = 600
    # The pane animates after a wheel event, so a resolved position is only
    # trustworthy once it repeats. Re-resolve until it holds still.
    settle_attempts: int = 6
    settle_s: float = 0.6


@dataclass
class Step:
    """One user action + its dual-oracle expectation."""

    name: str
    action: Action = field(default_factory=noop)
    # Resolved-at-run-time alternative to `action`. When set, `action` is
    # ignored: the two ways of saying "click this" must not both apply.
    target: Optional[TargetSpec] = None
    trigger: str = CHECKPOINT
    settle: bool = True
    # The visual question the assessor answers ("is the assistant reply
    # rendered and non-empty?"). None ⇒ visual oracle is skipped for this step.
    vision_question: Optional[str] = None
    # The kernel-side confirmation for this step. None ⇒ kernel oracle skipped.
    kernel_probe: Optional[KernelProbe] = None
    # Bound for POLL_UNTIL_VISIBLE.
    poll_timeout: float = 15.0
    # Crop the ASSESSOR'S copy of the frame to (x, y, w, h) before asking the
    # vision question (checkpoint steps only) — image tokens dominate assessor
    # cost, and a count/state question only needs its pane. The full frame is
    # still saved and kept as the forensic. None ⇒ assess the full frame.
    roi: Optional[Tuple[int, int, int, int]] = None


@dataclass
class Journey:
    name: str
    steps: List[Step] = field(default_factory=list)
