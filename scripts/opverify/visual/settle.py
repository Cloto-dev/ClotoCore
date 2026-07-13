"""Settle + poll primitives — the "capture only at the right moment" rules.

``settle`` waits for the screen to stop changing before a checkpoint capture,
so the VLM never assesses a mid-animation frame (which would make its verdict
non-deterministic — the seed of a flaky, rot-prone gate).

``poll_until_visible`` is the bounded trigger for asynchronous outcomes (a chat
reply that streams in, an install progress bar): it keeps looking until the
expected thing appears *or* a timeout — turning "the spinner never resolved"
into a caught, timestamped failure instead of a hang.

Both take injectable ``now`` / ``sleep`` so tests are deterministic (a fake
clock advances on each sleep) — no real waiting, no wall-clock flakiness.
"""

from __future__ import annotations

import time
from typing import Callable, Optional, Tuple

from .interfaces import Frame, ScreenSource, VisionAssessment, VisionAssessor


def settle(
    source: ScreenSource,
    *,
    stable_needed: int = 2,
    interval: float = 0.3,
    max_wait: float = 10.0,
    now: Callable[[], float] = time.monotonic,
    sleep: Callable[[float], None] = time.sleep,
) -> Frame:
    """Capture until ``stable_needed`` consecutive frames share a fingerprint
    (the UI stopped moving), or ``max_wait`` elapses. Returns the latest frame
    either way — a settle timeout still yields the freshest capture rather than
    raising, so the caller's oracle can judge whatever is on screen."""
    start = now()
    prev: Optional[Frame] = None
    run = 1
    while True:
        frame = source.grab()
        if prev is not None and frame.fingerprint == prev.fingerprint:
            run += 1
        else:
            run = 1
        if run >= stable_needed:
            return frame
        prev = frame
        if now() - start >= max_wait:
            return frame
        sleep(interval)


def poll_until_visible(
    source: ScreenSource,
    assessor: VisionAssessor,
    question: str,
    *,
    timeout: float = 15.0,
    interval: float = 0.5,
    now: Callable[[], float] = time.monotonic,
    sleep: Callable[[float], None] = time.sleep,
) -> Tuple[Frame, VisionAssessment, bool]:
    """Capture+assess repeatedly until the asked-for thing is visible or
    ``timeout`` elapses. Returns ``(last_frame, last_assessment, appeared)``.
    ``appeared=False`` is the stuck-spinner / never-rendered signal — precisely
    where the visual oracle beats the kernel oracle (the kernel may report the
    work done while the screen stays frozen)."""
    start = now()
    frame = source.grab()
    assessment = assessor.assess(frame, question)
    while not assessment.visible:
        if now() - start >= timeout:
            return frame, assessment, False
        sleep(interval)
        frame = source.grab()
        assessment = assessor.assess(frame, question)
    return frame, assessment, True
