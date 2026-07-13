"""Visual apex — agent-driven, user-emulation verification (opverify Phase 5).

Where the base :mod:`opverify` harness enters at the HTTP kernel (below the
GUI), this layer emulates a *human user*: it drives the real rendered GUI by
looking at pixels and acting on them, which is the highest-fidelity oracle
there is (nothing beats a real user actually using the app). Emulation, not
simulation — the only thing automated is the user.

The design authority is an earlier decision (the vision-apex line under an earlier decision).
The pieces here are deliberately **environment-independent**: they orchestrate
an abstract ``ScreenSource`` + ``Actuator`` + ``VisionAssessor`` and cross-check
against a ``KernelProbe``, so the whole loop is unit-testable with stubs before
any real VM / GUI is wired. Concrete backends (a Proxmox-VM interactive-session
actuator agent, a real VLM assessor) slot in behind those interfaces.

Core ideas, all realised here:

* **Dual oracle** (:mod:`.dual_oracle`) — each journey step is asserted twice:
  visually (did the user *see* it happen?) and at the kernel (did it *actually*
  take effect?). Divergence is not noise, it is the signal — a step the kernel
  confirms but the screen never shows is a **frontend bug**, the exact class a
  DOM-selector automation is blind to.
* **Tiered gate** — the kernel oracle is the hard gate (deterministic); the
  visual oracle starts soft/warn (VLM interpretation varies) and is promoted to
  hard only once proven stable ("build confidence first, flip to blocking once
  green").
* **Settle before capture** (:mod:`.settle`) — never assess a mid-transition
  frame; wait until the screen stops changing (pixel-fingerprint stability), so
  VLM interpretation stays deterministic.
* **Triggers** (:mod:`.driver`) — capture is event-driven at settle points, not
  fixed-interval: journey checkpoints, kernel events, bounded
  poll-until-visible (catches the stuck-spinner class), and failure-only
  forensic capture.
"""

from __future__ import annotations
