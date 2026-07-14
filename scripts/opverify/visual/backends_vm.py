"""Real backends that point the visual apex at a Windows VM running the app
under test through an interactive-desktop actuator agent (``opv_agent.py``).

Wiring (why it works across the Windows session boundary):

* The actuator agent runs **inside interactive session 1** (launched by a Task
  Scheduler Interactive principal), so mss can screenshot the real desktop and
  pyautogui can inject OS input. It listens on ``127.0.0.1:AGENT_PORT``.
* This driver runs on a separate host (a Mac) and reaches the VM over **SSH**.
  The SSH shell lands in **session 0**, which cannot screenshot session 1 — but
  it *can* open a localhost TCP socket, and that socket bridges to the agent in
  session 1. So every call here is ``ssh <vm> 'powershell … Invoke-WebRequest
  http://127.0.0.1:AGENT_PORT/…'`` — the network socket is the bridge the
  session wall does not block.
* The kernel oracle is the same trick pointed at the app's own kernel HTTP API
  (``127.0.0.1:KERNEL_PORT``): ``/api/system/health`` answers unauthenticated,
  which is exactly the deterministic hard-gate the dual oracle cross-checks the
  visual read against.

PowerShell is delivered via ``-EncodedCommand`` (UTF-16LE base64) so quoting is
immune to the Windows OpenSSH cmd.exe hop. Binary frames come back as base64 on
stdout (one round trip, no scp).

Config is env-driven (no secrets): ``OPV_VM_USER`` / ``OPV_VM_IP`` /
``OPV_AGENT_PORT`` (8900) / ``OPV_KERNEL_PORT`` (8081) / ``OPV_SSH_TIMEOUT``.
"""

from __future__ import annotations

import base64
import json
import os
import subprocess
from typing import List, Optional

from .interfaces import Action, Frame, VisionAssessment


def _cfg(name: str, default: str) -> str:
    return os.environ.get(name, default)


def _run_ps(ps: str, *, binary: bool = False, timeout: Optional[float] = None) -> bytes:
    """Run a PowerShell snippet on the VM via SSH -EncodedCommand; return stdout
    (raw bytes). Raises on non-zero exit."""
    vm = f"{_cfg('OPV_VM_USER', 'PC')}@{_cfg('OPV_VM_IP', '192.168.0.252')}"
    tmo = timeout if timeout is not None else float(_cfg("OPV_SSH_TIMEOUT", "25"))
    b64 = base64.b64encode(ps.encode("utf-16-le")).decode()
    proc = subprocess.run(
        ["ssh", "-o", "ConnectTimeout=8", "-o", "StrictHostKeyChecking=accept-new",
         vm, f"powershell -NoProfile -EncodedCommand {b64}"],
        capture_output=True, timeout=tmo,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"VM powershell failed ({proc.returncode}): "
            f"{proc.stderr.decode(errors='replace')[:400]}"
        )
    return proc.stdout


def _agent_url(path: str) -> str:
    return f"http://127.0.0.1:{_cfg('OPV_AGENT_PORT', '8900')}{path}"


def _kernel_url(path: str) -> str:
    return f"http://127.0.0.1:{_cfg('OPV_KERNEL_PORT', '8081')}{path}"


class VmAgentScreen:
    """ScreenSource: pulls a PNG from the agent's /grab, base64 over stdout."""

    def grab(self) -> Frame:
        url = _agent_url("/grab")
        ps = (
            "$ProgressPreference='SilentlyContinue';"
            f"$r = Invoke-WebRequest -Uri '{url}' -UseBasicParsing -TimeoutSec 15;"
            "[Convert]::ToBase64String($r.Content)"
        )
        out = _run_ps(ps).decode().strip()
        data = base64.b64decode(out)
        return Frame.of(data)


class VmAgentActuator:
    """Actuator: POSTs an Action to the agent's /act as JSON."""

    def send(self, action: Action) -> None:
        payload = {"kind": action.kind}
        for k in ("x", "y", "text", "key"):
            v = getattr(action, k)
            if v is not None:
                payload[k] = v
        body = json.dumps(payload).replace("'", "''")  # PS single-quote escape
        url = _agent_url("/act")
        ps = (
            "$ProgressPreference='SilentlyContinue';"
            f"$b = '{body}';"
            f"$r = Invoke-WebRequest -Uri '{url}' -Method POST -Body $b "
            "-ContentType 'application/json' -UseBasicParsing -TimeoutSec 15;"
            "$r.Content"
        )
        resp = _run_ps(ps).decode(errors="replace")
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
        url = _kernel_url(self._path)
        ps = (
            "$ProgressPreference='SilentlyContinue';"
            "try {"
            f"  (Invoke-WebRequest -Uri '{url}' -UseBasicParsing -TimeoutSec 5).Content"
            "} catch { 'HEALTH_ERR:' + $_.Exception.Message }"
        )
        body = _run_ps(ps).decode(errors="replace")
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
        url = _kernel_url(self._path)
        key = self._key.replace("'", "''")
        ps = (
            "$ProgressPreference='SilentlyContinue';"
            f"$h=@{{'X-API-Key'='{key}'}};"
            "try {"
            f"  (Invoke-WebRequest -Uri '{url}' -Headers $h -UseBasicParsing "
            "-TimeoutSec 8).Content"
            "} catch { 'API_ERR:'+$_.Exception.Message }"
        )
        body = _run_ps(ps).decode(errors="replace")
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
