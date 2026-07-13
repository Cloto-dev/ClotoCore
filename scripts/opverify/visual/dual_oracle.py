"""Dual oracle — cross-check the visual verdict against the kernel verdict.

This is the heart of the apex. Each journey step is judged twice:

* **visual** — did the user *see* the expected outcome on screen?
* **kernel** — did the operation *actually* take effect (opverify-style HTTP)?

Their agreement or disagreement is the diagnosis, and the disagreement cases
are the whole point — they localise the bug to a layer:

============  ============  ==================  ================================
kernel_ok     visual_ok     diagnosis           meaning
============  ============  ==================  ================================
True          True          AGREE_PASS          step genuinely succeeded
False         False         AGREE_FAIL          step genuinely failed (kernel)
True          False         FRONTEND_BUG        kernel did it, screen never
                                                 showed it → a GUI/render bug,
                                                 the class selector-automation
                                                 is blind to
False         True          BACKEND_OR_HIDDEN   screen showed it but the kernel
                                                 never confirmed → stale/faked
                                                 UI, or an optimistic render
============  ============  ==================  ================================

**Tiering** (D2 doctrine — kernel hard, visual soft-until-proven):

* ``hard_fail`` iff the kernel oracle failed. The kernel oracle is
  deterministic, so it gates.
* ``soft_fail`` iff the kernel passed but the visual oracle diverged
  (FRONTEND_BUG). This is a real, valuable finding, but the visual layer starts
  in warn tier — reported, not blocking — and is promoted to hard only once the
  vision assessor is proven stable enough not to false-positive.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum


class Diagnosis(str, Enum):
    AGREE_PASS = "agree_pass"
    AGREE_FAIL = "agree_fail"
    FRONTEND_BUG = "frontend_bug"
    BACKEND_OR_HIDDEN = "backend_or_hidden"


@dataclass
class Verdict:
    diagnosis: Diagnosis
    kernel_ok: bool
    visual_ok: bool
    hard_fail: bool
    soft_fail: bool
    note: str = ""

    @property
    def ok(self) -> bool:
        """A step is clean only when both oracles agree it passed."""
        return self.diagnosis is Diagnosis.AGREE_PASS


def cross_check(kernel_ok: bool, visual_ok: bool, note: str = "") -> Verdict:
    if kernel_ok and visual_ok:
        diag, hard, soft = Diagnosis.AGREE_PASS, False, False
    elif not kernel_ok and not visual_ok:
        diag, hard, soft = Diagnosis.AGREE_FAIL, True, False
    elif kernel_ok and not visual_ok:
        # Kernel confirms the effect but the user never saw it — the frontend
        # bug. Kernel is fine, so not a hard fail; the visual divergence is a
        # soft-tier finding until the vision oracle is promoted.
        diag, hard, soft = Diagnosis.FRONTEND_BUG, False, True
    else:  # visual_ok and not kernel_ok
        # The screen showed success the kernel never confirmed — a stale or
        # optimistic UI. The kernel oracle failed, so this hard-gates.
        diag, hard, soft = Diagnosis.BACKEND_OR_HIDDEN, True, False
    return Verdict(
        diagnosis=diag,
        kernel_ok=kernel_ok,
        visual_ok=visual_ok,
        hard_fail=hard,
        soft_fail=soft,
        note=note,
    )
