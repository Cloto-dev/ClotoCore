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
class Step:
    """One user action + its dual-oracle expectation."""

    name: str
    action: Action = field(default_factory=noop)
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
