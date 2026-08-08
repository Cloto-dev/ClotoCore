"""Abstract seams the visual apex is built on.

Everything here is a small dataclass or a ``Protocol`` — no I/O, no external
deps — so the orchestration in :mod:`.driver` can be exercised end-to-end with
the stubs in :mod:`.stub`, then repointed at real backends (a VM
interactive-session actuator, a real VLM) without touching the loop logic.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from typing import List, Optional, Protocol


# --------------------------------------------------------------------------
# Actions the emulated user can perform
# --------------------------------------------------------------------------
@dataclass
class Action:
    """A single user input. Coordinates are screen pixels (the actuator injects
    OS-level input); ``text``/``key`` carry keyboard payloads."""

    kind: str  # "click" | "type" | "key" | "move" | "scroll" | "noop"
    x: Optional[int] = None
    y: Optional[int] = None
    text: Optional[str] = None
    key: Optional[str] = None
    # Wheel delta for "scroll" (negative scrolls down). The agent scrolls at the
    # pointer, so a journey positions it with a "move" first — which pane is
    # under the cursor decides which pane moves.
    amount: Optional[int] = None


def click(x: int, y: int) -> Action:
    return Action("click", x=x, y=y)


def move(x: int, y: int) -> Action:
    return Action("move", x=x, y=y)


def scroll(amount: int) -> Action:
    return Action("scroll", amount=amount)


def type_text(text: str) -> Action:
    return Action("type", text=text)


def press_key(key: str) -> Action:
    return Action("key", key=key)


def noop() -> Action:
    return Action("noop")


# --------------------------------------------------------------------------
# Screen frames + fingerprints (settle detection needs a cheap "did it
# change?" signal without decoding the image)
# --------------------------------------------------------------------------
def _png_size(data: bytes):
    """(width, height) of a PNG, or (None, None) for anything else.

    The size of the picture is how the harness knows which pixel space it is
    looking at -- a DPI-unaware capture of a 1280x800 panel comes back 1024x640
    at 125 % scale, and nothing else in a frame says so. Stub frames are not
    PNGs, so this stays quiet rather than raising for them.
    """
    if len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n":
        return None, None
    try:
        return (
            int.from_bytes(data[16:20], "big"),
            int.from_bytes(data[20:24], "big"),
        )
    except Exception:
        return None, None


@dataclass
class Frame:
    """A captured screen image plus a cheap fingerprint used by settle to
    decide the UI has stopped moving. A real ``ScreenSource`` may fingerprint by
    exact hash (below) or by a perceptual/threshold hash to tolerate a blinking
    cursor — :mod:`.settle` only compares fingerprints, so either works."""

    data: bytes
    fingerprint: str
    width: Optional[int] = None
    height: Optional[int] = None

    @classmethod
    def of(
        cls, data: bytes, width: Optional[int] = None, height: Optional[int] = None
    ) -> "Frame":
        if width is None and height is None:
            width, height = _png_size(data)
        return cls(
            data=data,
            fingerprint=hashlib.sha256(data).hexdigest(),
            width=width,
            height=height,
        )


class ScreenSource(Protocol):
    """Grabs the current screen of the app under test."""

    def grab(self) -> Frame: ...


class Actuator(Protocol):
    """Injects a user input into the interactive session of the app under
    test (real backend: a VM interactive-desktop agent driving mss/pyautogui)."""

    def send(self, action: Action) -> None: ...


# --------------------------------------------------------------------------
# Vision oracle — "look at this frame; is the expected thing there?"
# --------------------------------------------------------------------------
@dataclass
class VisionAssessment:
    """What the VLM saw when asked a yes/no visual question about a frame."""

    visible: bool
    detail: str = ""
    defects: List[str] = field(
        default_factory=list
    )  # overlap / cutoff / broken-render …


class VisionAssessor(Protocol):
    """Answers a visual question about a frame. The real backend is a
    multimodal model looking at the screenshot the way a user would; the stub
    scripts the answers so the loop is deterministic in tests."""

    def assess(self, frame: Frame, question: str) -> VisionAssessment: ...


# --------------------------------------------------------------------------
# "I could not ask" — distinct from "the answer is no"
# --------------------------------------------------------------------------
class ProbeUnavailable(RuntimeError):
    """The kernel could not be *asked*, so nothing was learned about its state.

    The distinction this class carries is the whole point of it. A kernel that
    answers 403 because the key is wrong returns a body with no ``providers``
    key in it, and ``body.get("providers", [])`` turns that into "this machine
    has no providers" — an absent *state* rather than an absent *answer*. On
    2026-08-06 that read a fully configured VM as an unmet fixture and printed
    the remedy for an unmet fixture, which is a rollback that would have
    discarded the very state it failed to see (bug-500).

    So: raise this when the question could not be put to the kernel (non-2xx,
    unset credentials), and let a genuine "no" be an ordinary False. Callers
    that treat both the same are the bug.
    """

    def __init__(self, message: str, status: Optional[int] = None):
        super().__init__(message)
        self.status = status


# --------------------------------------------------------------------------
# Kernel oracle — the deterministic hard-gate cross-check (opverify-style)
# --------------------------------------------------------------------------
class KernelProbe(Protocol):
    """Confirms, over the kernel's HTTP API, that a step actually took effect
    underneath the GUI. Returns True iff the kernel state confirms it. This is
    the hard gate; the visual oracle is cross-checked against it."""

    def check(self) -> bool: ...
