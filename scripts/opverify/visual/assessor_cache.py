"""Cut redundant multimodal assessments (#236). A live assessor (a Sonnet
VM-executor reading the frame) costs tokens + latency per call, and a
poll-until-visible checkpoint asks the same visual question of many frames while
it waits — often the *same* frame, byte for byte, when the screen is frozen.

:class:`CachingAssessor` memoizes the verdict by ``(frame.fingerprint, question)``
and returns it without re-invoking the inner assessor when that exact frame+
question repeats. Byte-identical frames must yield the same visual answer, so
this is outcome-preserving — it only removes calls, never changes a verdict.
The fingerprint is already computed at grab time, so the change-detection is
free.

Only wrap *stateless* assessors. It must NOT wrap :class:`RecordedVision`, whose
answers are consumed in call order — skipping a call would misalign the recorded
sequence. It is meant for the live handshake assessor / a live VLM.
"""

from __future__ import annotations

from typing import Dict, Tuple

from .interfaces import Frame, VisionAssessment, VisionAssessor


class CachingAssessor:
    def __init__(self, inner: VisionAssessor):
        self._inner = inner
        self._cache: Dict[Tuple[str, str], VisionAssessment] = {}
        self.inner_calls = 0  # observability: how many real assessments happened

    def assess(self, frame: Frame, question: str) -> VisionAssessment:
        key = (frame.fingerprint, question)
        cached = self._cache.get(key)
        if cached is not None:
            return cached
        result = self._inner.assess(frame, question)
        self.inner_calls += 1
        self._cache[key] = result
        return result
