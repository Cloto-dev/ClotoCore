"""Live vision oracles that replace the :class:`RecordedVision` bootstrap — the
apex's "the AI perceives like a user" closed with a *live* multimodal read
instead of pre-recorded answers.

:class:`AgentHandshakeAssessor` is the 2-tier executor's eyes made pluggable
into the structured driver loop (journey + dual-oracle + tiering + forensic).
Each :meth:`assess` drops the frame + question into an exchange dir and blocks
until the driving agent — a Sonnet VM-executor subagent, unattended AI, no API
key — writes the matching verdict. So a subagent can drive a whole journey with
its own multimodal read rather than hand-rolling raw curl I/O.

Exchange protocol (files in ``exchange_dir``, all writes atomic via tmp+rename):
  driver writes   ``frame_NNN.png``           the captured frame
  driver writes   ``req_NNN.json``  {seq, question, frame}
  agent  writes   ``resp_NNN.json`` {visible: bool, detail: str, defects: [str]}
The agent watches for ``req_*.json`` with no matching ``resp_*.json``, reads the
referenced PNG, and writes its verdict. ``done.flag`` (written by the driver on
exit) tells the responder loop to stop.
"""

from __future__ import annotations

import json
import os
import time
from typing import Callable

from .interfaces import Frame, VisionAssessment


class AgentHandshakeAssessor:
    """VisionAssessor answered live by the driving agent through a file
    exchange. Blocking: :meth:`assess` returns only once the agent has written
    the verdict for that frame (or raises on timeout)."""

    def __init__(
        self,
        exchange_dir: str,
        *,
        poll: float = 0.5,
        timeout: float = 240.0,
        now: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], None] = time.sleep,
    ):
        self._dir = exchange_dir
        os.makedirs(exchange_dir, exist_ok=True)
        self._poll = poll
        self._timeout = timeout
        self._now = now
        self._sleep = sleep
        self._seq = 0

    def _atomic_write(self, path: str, data: bytes) -> None:
        tmp = f"{path}.tmp"
        with open(tmp, "wb") as f:
            f.write(data)
        os.replace(tmp, path)

    def assess(self, frame: Frame, question: str) -> VisionAssessment:
        seq = self._seq
        self._seq += 1
        frame_path = os.path.join(self._dir, f"frame_{seq:03d}.png")
        self._atomic_write(frame_path, frame.data)
        self._atomic_write(
            os.path.join(self._dir, f"req_{seq:03d}.json"),
            json.dumps(
                {"seq": seq, "question": question, "frame": frame_path}
            ).encode(),
        )
        resp_path = os.path.join(self._dir, f"resp_{seq:03d}.json")
        deadline = self._now() + self._timeout
        while self._now() < deadline:
            if os.path.exists(resp_path):
                with open(resp_path, "rb") as f:
                    d = json.loads(f.read())
                return VisionAssessment(
                    visible=bool(d["visible"]),
                    detail=str(d.get("detail", "")),
                    defects=list(d.get("defects", [])),
                )
            self._sleep(self._poll)
        raise TimeoutError(
            f"no verdict for req_{seq:03d} in {self._dir} within {self._timeout}s"
        )

    def signal_done(self) -> None:
        """Tell the agent's responder loop the run is over (written on teardown)."""
        self._atomic_write(os.path.join(self._dir, "done.flag"), b"1")
