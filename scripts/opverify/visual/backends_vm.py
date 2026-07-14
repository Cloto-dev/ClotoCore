"""Real backends that point the visual apex at a Windows VM running the app
under test through an interactive-desktop actuator agent (``opv_agent.py``).

Wiring (why it works across the Windows session boundary):

* The actuator agent runs **inside interactive session 1** (launched by a Task
  Scheduler Interactive principal), so mss can screenshot the real desktop and
  pyautogui can inject OS input. It listens on ``127.0.0.1:AGENT_PORT``.
* This driver runs on a separate host (a Mac) and reaches the VM over **SSH**.
  The SSH shell lands in **session 0**, which cannot screenshot session 1 — but
  it *can* open a localhost TCP socket, and that socket bridges to the agent in
  session 1. So every call here is ``ssh <vm> 'curl.exe
  http://127.0.0.1:AGENT_PORT/…'`` — the network socket is the bridge the
  session wall does not block.
* The kernel oracle is the same trick pointed at the app's own kernel HTTP API
  (``127.0.0.1:KERNEL_PORT``): ``/api/system/health`` answers unauthenticated,
  which is exactly the deterministic hard-gate the dual oracle cross-checks the
  visual read against.

Transport: every call is one ``ssh <vm> 'curl.exe …'``. The VM's default
OpenSSH shell is PowerShell, and bare ``curl.exe`` (shipped with Windows 10+)
needs no ``-EncodedCommand`` wrapper — dropping PowerShell startup from the hot
path. GET bodies come straight back (PNG frames as *raw bytes* — no base64, no
scp); POST bodies ride ``--data-binary '@-'`` over ssh stdin (the ``'@-'`` is
PS-quoted so PowerShell passes it to curl instead of splatting it), which is
immune to shell quoting. SSH connection multiplexing (``ControlMaster`` +
``ControlPersist``) keeps one shared connection warm for the whole run, so only
the first call pays the TCP+auth handshake — measured ~0.76s→~0.4s per call.

Checkpoint fusion (:class:`CompositeVmScreen` + :class:`CoFetchHealthProbe`): a
checkpoint needs the frame *and* the kernel liveness gate. curl chains both in a
single process (``--next``) with a ``-w`` delimiter between the bodies, so the
two are one ssh round trip instead of two — halving per-checkpoint latency on
top of the multiplexing win.

Config is env-driven (no secrets): ``OPV_VM_USER`` / ``OPV_VM_IP`` /
``OPV_AGENT_PORT`` (8900) / ``OPV_KERNEL_PORT`` (8081) / ``OPV_SSH_TIMEOUT`` /
``OPV_SSH_PERSIST`` (ControlPersist seconds, default 60) / ``OPV_SSH_CONTROL_PATH``.
"""

from __future__ import annotations

import json
import os
import subprocess
from typing import List, Optional

from .interfaces import Action, Frame, VisionAssessment


def _cfg(name: str, default: str) -> str:
    return os.environ.get(name, default)


def _ssh_cmd() -> List[str]:
    """SSH invocation prefix with connection multiplexing — the master
    connection persists ``OPV_SSH_PERSIST`` seconds so subsequent calls reuse
    it and skip the TCP+auth handshake (the dominant per-call cost)."""
    vm = f"{_cfg('OPV_VM_USER', 'PC')}@{_cfg('OPV_VM_IP', '192.0.2.252')}"
    return [
        "ssh",
        "-o",
        "ConnectTimeout=8",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        "ControlMaster=auto",
        "-o",
        f"ControlPersist={_cfg('OPV_SSH_PERSIST', '60')}",
        "-o",
        f"ControlPath={_cfg('OPV_SSH_CONTROL_PATH', '/tmp/opv-ssh-%r@%h:%p')}",
        vm,
    ]


def _run(
    remote: str, *, stdin: Optional[bytes] = None, timeout: Optional[float] = None
) -> bytes:
    """Run one remote command over the multiplexed SSH connection; return
    stdout (raw bytes). Raises on non-zero exit."""
    tmo = timeout if timeout is not None else float(_cfg("OPV_SSH_TIMEOUT", "25"))
    proc = subprocess.run(
        _ssh_cmd() + [remote], input=stdin, capture_output=True, timeout=tmo
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"VM ssh failed ({proc.returncode}): "
            f"{proc.stderr.decode(errors='replace')[:400]}"
        )
    return proc.stdout


def _agent_url(path: str) -> str:
    return f"http://127.0.0.1:{_cfg('OPV_AGENT_PORT', '8900')}{path}"


def _kernel_url(path: str) -> str:
    return f"http://127.0.0.1:{_cfg('OPV_KERNEL_PORT', '8081')}{path}"


# --- grab + liveness co-fetch (one round trip) ---------------------------
# A checkpoint needs both the frame (agent /grab) and the kernel liveness gate
# (/api/system/health). Done naively that is two SSH round trips. curl can chain
# both transfers in ONE process (``--next``) and emit a delimiter between the two
# bodies (``-w``), so the whole checkpoint costs a single ssh — and because it is
# still one ``curl.exe`` process, its stdout streams raw exactly like the plain
# /grab (PR #339: byte-identical PNG). The frame comes first (the agent is always
# up, so its ``-w`` reliably prints the delimiter); the health body follows and
# may be empty if the kernel is unreachable (→ a False gate, which is correct).
_DELIM_TOKEN = "--OPV-COFETCH-9d1f7--"
# curl -w interprets the literal ``\n`` escapes; the marker is wrapped in
# newlines so it never abuts binary PNG bytes. Single-quoted at the call site so
# the PowerShell default shell passes it to curl verbatim.
_DELIM_WRITEOUT = rf"\n{_DELIM_TOKEN}\n"
_DELIM_BYTES = f"\n{_DELIM_TOKEN}\n".encode()


def _split_composite(raw: bytes) -> tuple:
    """Split a fused ``grab -w DELIM --next health`` stream into
    ``(png_bytes, health_body)``. Uses ``rfind`` so that even in the
    astronomically unlikely case the PNG contains the delimiter byte sequence,
    the true separator (the last one — the trailing health body carries no
    delimiter) is the one chosen. No delimiter at all → treat the whole payload
    as the frame with no health co-fetched."""
    idx = raw.rfind(_DELIM_BYTES)
    if idx == -1:
        return raw, b""
    return raw[:idx], raw[idx + len(_DELIM_BYTES) :]


class _CoFetchCell:
    """One-slot cache shared by a :class:`CompositeVmScreen` and its paired
    :class:`CoFetchHealthProbe`: a grab drops the co-fetched health body here and
    the probe consumes it, so the liveness gate costs no round trip of its own.
    The driver always grabs before it probes within a step, so the body a probe
    reads is always the one this step's own grab just fetched."""

    def __init__(self) -> None:
        self.health_body: Optional[bytes] = None

    def put(self, body: bytes) -> None:
        self.health_body = body


class VmAgentScreen:
    """ScreenSource: pulls the PNG from the agent's /grab as raw bytes (curl
    streams the binary body straight back over ssh — no base64 round trip)."""

    def grab(self) -> Frame:
        data = _run(f"curl.exe -s -m 15 {_agent_url('/grab')}")
        return Frame.of(data)


class VmAgentActuator:
    """Actuator: POSTs an Action to the agent's /act as JSON."""

    def send(self, action: Action) -> None:
        payload = {"kind": action.kind}
        for k in ("x", "y", "text", "key"):
            v = getattr(action, k)
            if v is not None:
                payload[k] = v
        # Body rides ssh stdin via curl --data-binary '@-' (the '@-' is single-
        # quoted so the PowerShell default shell passes it to curl literally
        # instead of splatting it) — no shell quoting of the JSON at all.
        body = json.dumps(payload).encode()
        resp = _run(
            f"curl.exe -s -m 15 -X POST {_agent_url('/act')} --data-binary '@-'",
            stdin=body,
        ).decode(errors="replace")
        if '"ok": true' not in resp and '"ok":true' not in resp:
            raise RuntimeError(f"/act rejected: {resp[:200]}")


class KernelHealthProbe:
    """KernelProbe: the deterministic hard-gate. True iff the kernel's
    /api/system/health reports status ok. This is the unauthenticated liveness
    gate; operation-level probes (agents/history) need the app's X-API-Key and
    are layered on top later."""

    def __init__(self, path: str = "/api/system/health", want: str = '"status":"ok"'):
        self._path = path
        self._want = want

    def check(self) -> bool:
        try:
            body = _run(
                f"curl.exe -s -m 5 {_kernel_url(self._path)}", timeout=12
            ).decode(errors="replace")
        except Exception:  # noqa: BLE001 - unreachable kernel is a False gate
            return False
        return self._want in body.replace(" ", "")


class KernelApiProbe:
    """Operation-level kernel oracle: an *authenticated* GET whose response body
    must contain ``want``. Unlike :class:`KernelHealthProbe` (unauthenticated
    liveness), the admin routes (``/api/agents``, ``/api/history`` …) return 403
    without the app's ``X-API-Key``, so this probe carries the key the harness
    set via ``CLOTO_API_KEY`` when it launched the GUI (read from ``OPV_API_KEY``
    by default). This lifts the hard-gate from "is the kernel alive" to "did the
    operation actually take effect underneath the GUI" (an agent exists, a chat
    turn persisted)."""

    def __init__(self, path: str, want: str, api_key: Optional[str] = None):
        self._path = path
        self._want = want
        self._key = api_key if api_key is not None else _cfg("OPV_API_KEY", "")

    def check(self) -> bool:
        # key is a hex token (secrets.token_hex) — safe inside single quotes.
        try:
            body = _run(
                f"curl.exe -s -m 8 -H 'X-API-Key: {self._key}' "
                f"{_kernel_url(self._path)}",
                timeout=15,
            ).decode(errors="replace")
        except Exception:  # noqa: BLE001 - unreachable kernel is a False gate
            return False
        return self._want in body


class CompositeVmScreen:
    """ScreenSource that co-fetches the kernel liveness body in the SAME ssh
    command as the /grab (curl ``--next``), halving a checkpoint's round trips
    (frame + liveness gate) from two to one. Byte-for-byte the same PNG as
    :class:`VmAgentScreen` — it is still a single ``curl.exe`` process, so stdout
    streams raw — plus a trailing health body it drops into the shared cell for
    the paired :class:`CoFetchHealthProbe` to consume."""

    def __init__(
        self, cell: _CoFetchCell, health_path: str = "/api/system/health"
    ) -> None:
        self._cell = cell
        self._health_path = health_path

    def grab(self) -> Frame:
        raw = _run(
            f"curl.exe -s -m 15 {_agent_url('/grab')} -w '{_DELIM_WRITEOUT}' "
            f"--next -s -m 5 {_kernel_url(self._health_path)}"
        )
        png, health = _split_composite(raw)
        self._cell.put(health)
        return Frame.of(png)


class VmAgentHashSource:
    """A cheap change-signal for :func:`settle` polling: hits the agent's
    ``/grabhash`` (a hash of the raw framebuffer — no PNG encode, no ~85 KB
    body) and returns the hash string. Wire ``VmAgentHashSource().hash`` as the
    driver's ``change_probe`` so settling costs N tiny hash polls plus one real
    grab instead of N full-frame grabs. Requires agent version ≥ 3."""

    def hash(self) -> str:
        body = _run(f"curl.exe -s -m 10 {_agent_url('/grabhash')}").decode(
            errors="replace"
        )
        try:
            return json.loads(body)["hash"]
        except (ValueError, KeyError) as e:  # 404 (old agent) or malformed
            raise RuntimeError(
                f"/grabhash unavailable (agent < v3?): {body[:120]}"
            ) from e


class CoFetchHealthProbe:
    """KernelProbe (liveness hard-gate) that reads the health body co-fetched by
    the paired :class:`CompositeVmScreen`'s most recent grab — zero extra ssh.
    Falls back to a standalone health request only if no grab has populated the
    cell yet (a probe called before any grab, which the driver never does)."""

    def __init__(
        self,
        cell: _CoFetchCell,
        path: str = "/api/system/health",
        want: str = '"status":"ok"',
    ):
        self._cell = cell
        self._path = path
        self._want = want

    def check(self) -> bool:
        body = self._cell.health_body
        if body is None:
            try:
                body = _run(f"curl.exe -s -m 5 {_kernel_url(self._path)}", timeout=12)
            except Exception:  # noqa: BLE001 - unreachable kernel is a False gate
                return False
        return self._want in body.decode(errors="replace").replace(" ", "")


def make_cofetch_backend(
    health_path: str = "/api/system/health", want: str = '"status":"ok"'
) -> tuple:
    """Build a ``(screen, health_probe)`` pair sharing a cell so that grab +
    liveness fuse into one ssh round trip. Wire the screen into the driver and
    hand the probe to every health-gated journey step."""
    cell = _CoFetchCell()
    return (
        CompositeVmScreen(cell, health_path),
        CoFetchHealthProbe(cell, health_path, want),
    )


class RecordedVision:
    """VisionAssessor whose answers were produced by the human/agent multimodal
    read of this run's frames, consumed in call order. This is the honest
    bootstrap oracle: the apex's vision layer is a real intelligent perceiver
    (the agent looking at the screenshot), recorded here so the driver run is
    reproducible. Swap for a live multimodal-model assessor to fully automate.

    Each entry: {"visible": bool, "detail": str, "defects": [str]}.
    """

    def __init__(self, sequence: List[dict]):
        self._seq = list(sequence)
        self._i = 0

    def assess(self, frame: Frame, question: str) -> VisionAssessment:
        if self._i >= len(self._seq):
            raise RuntimeError("RecordedVision exhausted — more asks than recorded")
        e = self._seq[self._i]
        self._i += 1
        return VisionAssessment(
            visible=bool(e["visible"]),
            detail=e.get("detail", ""),
            defects=list(e.get("defects", [])),
        )
