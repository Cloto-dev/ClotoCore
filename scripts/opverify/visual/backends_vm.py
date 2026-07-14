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
    vm = f"{_cfg('OPV_VM_USER', 'PC')}@{_cfg('OPV_VM_IP', '192.168.0.252')}"
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
