"""Deterministic stubs for the abstract seams, so the whole orchestration loop
can be exercised without a real GUI / VM / VLM.

* ``ScriptedScreen`` — returns frames from a scripted sequence (repeats the last
  once exhausted, so settle converges on a stable tail).
* ``RecordingActuator`` — records the actions sent, never fails.
* ``ScriptedVision`` — answers the visual question from a ``{fingerprint: bool}``
  map (or a callable), with optional detail/defects.
* ``ScriptedKernel`` — a ``KernelProbe`` returning a scripted / callable bool.
* ``FakeClock`` — a monotonic clock that only advances when its ``sleep`` is
  called, so timeouts are deterministic and instant.
"""

from __future__ import annotations

from typing import Callable, Dict, List, Optional, Union

from .interfaces import Action, Frame, VisionAssessment


class ScriptedScreen:
    def __init__(self, frames: List[Frame]):
        assert frames, "ScriptedScreen needs at least one frame"
        self._frames = frames
        self._i = 0

    def grab(self) -> Frame:
        frame = self._frames[min(self._i, len(self._frames) - 1)]
        self._i += 1
        return frame


class RecordingActuator:
    def __init__(self) -> None:
        self.actions: List[Action] = []

    def send(self, action: Action) -> None:
        self.actions.append(action)


class ScriptedVision:
    """Map a frame fingerprint → visible bool. ``default`` covers frames not in
    the map. A callable ``script`` can express richer logic."""

    def __init__(
        self,
        script: Union[Dict[str, bool], Callable[[Frame, str], bool]],
        *,
        default: bool = False,
        detail: str = "",
        defects: Optional[List[str]] = None,
    ):
        self._script = script
        self._default = default
        self._detail = detail
        self._defects = defects or []

    def assess(self, frame: Frame, question: str) -> VisionAssessment:
        if callable(self._script):
            visible = bool(self._script(frame, question))
        else:
            visible = self._script.get(frame.fingerprint, self._default)
        return VisionAssessment(
            visible=visible, detail=self._detail, defects=list(self._defects)
        )


class ScriptedKernel:
    def __init__(self, result: Union[bool, Callable[[], bool]]):
        self._result = result

    def check(self) -> bool:
        return bool(self._result() if callable(self._result) else self._result)


class FakeClock:
    """Advances only on ``sleep`` — deterministic, no wall-clock waiting."""

    def __init__(self) -> None:
        self.t = 0.0

    def now(self) -> float:
        return self.t

    def sleep(self, dt: float) -> None:
        self.t += dt


def frame(tag: str) -> Frame:
    """A tiny distinct frame whose bytes (hence fingerprint) key off ``tag``."""
    return Frame.of(tag.encode("utf-8"))
