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

Faster transport (:class:`SshTunnel` + ``Tunnel*`` backends, #235): even a warm
``ssh 'curl.exe …'`` pays ~350ms/call to open an exec channel and spawn
PowerShell + curl on the VM. An SSH local port-forward pays that once — a
persistent master forwards the agent/kernel loopback ports to local ports and
the Mac hits them with a plain local HTTP client. Measured ~2.3x on /grab and
kernel probes from ~350ms to ~7ms; ``run_vm`` uses it by default
(``OPV_TRANSPORT=curl`` selects the ssh+curl transport above). The remaining
/grab floor is the VM's mss screen capture, which no transport can remove.

Config is env-driven (no secrets): ``OPV_VM_USER`` / ``OPV_VM_IP`` /
``OPV_AGENT_PORT`` (8900) / ``OPV_KERNEL_PORT`` (8081) / ``OPV_SSH_TIMEOUT`` /
``OPV_SSH_PERSIST`` (ControlPersist seconds, default 60) / ``OPV_SSH_CONTROL_PATH``.
"""

from __future__ import annotations

import http.client
import json
import os
import socket
import subprocess
import time
from typing import List, Optional

from .interfaces import Action, Frame, ProbeUnavailable, VisionAssessment


def _cfg(name: str, default: str) -> str:
    return os.environ.get(name, default)


def _split_status(raw: bytes) -> tuple:
    """Split ``curl -w '\\n%{http_code}'`` output into (body, status).

    The status is appended after a newline, so the body is everything before
    the last one. A reply that carries no parseable trailer is reported as
    status 0 — unknown, which the callers treat as "could not ask" rather than
    silently as 200.
    """
    head, sep, tail = raw.rpartition(b"\n")
    if not sep:
        return raw, 0
    try:
        return head, int(tail.strip())
    except ValueError:
        return raw, 0


def _require_key(key: str, path: str) -> str:
    """Refuse to send an authenticated probe with no credential.

    ``OPV_API_KEY`` defaults to the empty string, so forgetting it does not
    fail — it sends a keyless request, collects the 403, and hands back a body
    with none of the keys the caller reads. The run then reports the *state*
    as absent. Failing here names the real cause once, instead of letting it
    surface as a fixture the machine appears not to be in (bug-500).
    """
    if not key:
        raise ProbeUnavailable(
            f"OPV_API_KEY is unset, so the authenticated probe of {path} was never "
            "actually put to the kernel. Set it to the CLOTO_API_KEY the app under "
            "test was launched with — an empty key returns 403, and a 403 body "
            "reads as an absent state, not as a failure to ask."
        )
    return key


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
        for k in ("x", "y", "text", "key", "amount"):
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
        key = _require_key(self._key, self._path)
        try:
            raw = _run(
                f"curl.exe -s -m 8 -w '\\n%{{http_code}}' -H 'X-API-Key: {key}' "
                f"{_kernel_url(self._path)}",
                timeout=15,
            )
        except Exception:  # noqa: BLE001 - unreachable kernel is a False gate
            return False
        body, status = _split_status(raw)
        # A rejected credential is not the kernel saying "no" (bug-500).
        if not 200 <= status < 300:
            raise ProbeUnavailable(
                f"kernel answered {status} for {self._path} "
                f"({'check OPV_API_KEY' if status in (401, 403) else 'not a state answer'})",
                status=status,
            )
        return self._want in body.decode(errors="replace")


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


# --- persistent SSH port-forward transport (#235) ------------------------
# The dominant per-call cost of the ssh+curl.exe transport is NOT the payload —
# it is opening an ssh exec channel and spawning PowerShell + curl.exe on the VM
# for *every* call (~350ms floor even with a warm ControlMaster). An SSH local
# port-forward pays that once: a persistent master forwards the VM's agent (8900)
# and kernel (8081) loopback ports to local ports, and the Mac then hits them
# with a plain local HTTP client — no per-call ssh exec, no PowerShell, no remote
# curl. Measured on VM104: /grab 606→259ms (2.3x), kernel probe ~350→7.5ms. The
# ~258ms /grab floor that remains is the VM's mss screen capture (grab ≈ grabhash
# over the tunnel), which this transport cannot touch.


def _free_local_port() -> int:
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]
    finally:
        s.close()


def _http_get(port: int, path: str, headers=None, timeout: float = 15.0) -> bytes:
    """GET over the port-forward, refusing to return a body the kernel did not
    agree to give.

    The status check lives here rather than at the call sites because here is
    the only place that still holds the response. Return the body and every
    caller downstream sees an ordinary dict with the keys missing — which is
    how a 403 became "this machine has no providers" (bug-500).
    """
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    try:
        conn.request("GET", path, headers=headers or {})
        resp = conn.getresponse()
        body = resp.read()
        if not 200 <= resp.status < 300:
            raise ProbeUnavailable(
                f"kernel answered {resp.status} for {path} "
                f"({'check OPV_API_KEY' if resp.status in (401, 403) else 'not a state answer'}): "
                f"{body[:120].decode(errors='replace')!r}",
                status=resp.status,
            )
        return body
    finally:
        conn.close()


def _http_post(port: int, path: str, body: bytes, timeout: float = 15.0) -> bytes:
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    try:
        conn.request(
            "POST", path, body=body, headers={"Content-Type": "application/json"}
        )
        return conn.getresponse().read()
    finally:
        conn.close()


class SshTunnel:
    """A persistent SSH master that port-forwards the VM's agent + kernel
    loopback ports to local ports, so the whole run reaches them over plain local
    HTTP instead of a per-call ``ssh 'curl.exe …'``. Open once, use many times,
    close on teardown (context-manager friendly). ``local_agent`` / ``local_kernel``
    are the Mac-side ports the backends target."""

    def __init__(
        self,
        agent_port: Optional[int] = None,
        kernel_port: Optional[int] = None,
    ):
        self.vm = f"{_cfg('OPV_VM_USER', 'PC')}@{_cfg('OPV_VM_IP', '192.0.2.252')}"
        self.agent_port = agent_port or int(_cfg("OPV_AGENT_PORT", "8900"))
        self.kernel_port = kernel_port or int(_cfg("OPV_KERNEL_PORT", "8081"))
        self.local_agent = _free_local_port()
        self.local_kernel = _free_local_port()
        self._ctl = f"/tmp/opv-tunnel-{self.local_agent}-%r@%h:%p"

    def open(self, timeout: float = 20.0) -> "SshTunnel":
        subprocess.run(
            [
                "ssh",
                "-f",
                "-N",
                "-M",
                "-o",
                f"ControlPath={self._ctl}",
                "-o",
                f"ControlPersist={_cfg('OPV_SSH_PERSIST', '300')}",
                "-o",
                "ConnectTimeout=8",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-L",
                f"{self.local_agent}:127.0.0.1:{self.agent_port}",
                "-L",
                f"{self.local_kernel}:127.0.0.1:{self.kernel_port}",
                self.vm,
            ],
            check=True,
            timeout=timeout,
            capture_output=True,
        )
        # Wait until the agent forward actually answers (the master returns from
        # -f before the remote side is necessarily reachable through it).
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                _http_get(self.local_agent, "/health", timeout=2.0)
                return self
            except OSError:
                time.sleep(0.2)
        self.close()
        raise RuntimeError("ssh tunnel forwards did not come up within timeout")

    def close(self) -> None:
        subprocess.run(
            ["ssh", "-O", "exit", "-o", f"ControlPath={self._ctl}", self.vm],
            capture_output=True,
        )

    def __enter__(self) -> "SshTunnel":
        return self.open()

    def __exit__(self, *exc) -> None:
        self.close()


class TunnelScreen:
    """ScreenSource over the port-forward: a plain local HTTP GET /grab (raw PNG,
    byte-identical to the ssh+curl transport)."""

    def __init__(self, tunnel: SshTunnel):
        self._t = tunnel

    def grab(self) -> Frame:
        return Frame.of(_http_get(self._t.local_agent, "/grab"))


class TunnelActuator:
    """Actuator over the port-forward: POST /act as JSON on the local HTTP."""

    def __init__(self, tunnel: SshTunnel):
        self._t = tunnel

    def send(self, action: Action) -> None:
        payload = {"kind": action.kind}
        for k in ("x", "y", "text", "key", "amount"):
            v = getattr(action, k)
            if v is not None:
                payload[k] = v
        resp = _http_post(
            self._t.local_agent, "/act", json.dumps(payload).encode()
        ).decode(errors="replace")
        if '"ok": true' not in resp and '"ok":true' not in resp:
            raise RuntimeError(f"/act rejected: {resp[:200]}")


class TunnelHealthProbe:
    """KernelProbe (liveness) over the port-forward — a ~7ms local HTTP GET
    (vs ~350ms for the ssh+curl transport), so co-fetching it into the grab is no
    longer worth the complexity; it is just its own cheap call."""

    def __init__(
        self,
        tunnel: SshTunnel,
        path: str = "/api/system/health",
        want: str = '"status":"ok"',
    ):
        self._t = tunnel
        self._path = path
        self._want = want

    def check(self) -> bool:
        try:
            body = _http_get(self._t.local_kernel, self._path, timeout=8.0).decode(
                errors="replace"
            )
        # Liveness is the one probe for which "could not ask" and "the answer
        # is no" really are the same thing: the question is whether the kernel
        # is answering at all. Unauthenticated, so a 403 here is not the
        # credential trap the authenticated probes guard against.
        except (OSError, ProbeUnavailable):  # unreachable kernel is a False gate
            return False
        return self._want in body.replace(" ", "")


class TunnelApiProbe:
    """Operation-level authenticated kernel oracle over the port-forward."""

    def __init__(
        self,
        tunnel: SshTunnel,
        path: str,
        want: str,
        api_key: Optional[str] = None,
    ):
        self._t = tunnel
        self._path = path
        self._want = want
        self._key = api_key if api_key is not None else _cfg("OPV_API_KEY", "")

    def check(self) -> bool:
        # A missing or rejected credential is not a "no" from the kernel, so it
        # must not become one: ProbeUnavailable propagates out of the poll loop
        # and fails the run loudly (bug-500). Only an unreachable socket is a
        # False gate, as before.
        headers = {"X-API-Key": _require_key(self._key, self._path)}
        try:
            body = _http_get(
                self._t.local_kernel,
                self._path,
                headers=headers,
                timeout=15.0,
            ).decode(errors="replace")
        except OSError:
            return False
        return self._want in body


class TunnelJsonFetch:
    """Authenticated kernel JSON GET over the port-forward, for journey
    construction that derives its visual expectations from the kernel at run
    time — e.g. the danger-zone questions embed the exact entry count the plan
    endpoint reports, so the question text cannot go stale when the plan
    changes shape."""

    def __init__(self, tunnel: SshTunnel, api_key: Optional[str] = None):
        self._t = tunnel
        self._key = api_key if api_key is not None else _cfg("OPV_API_KEY", "")

    def __call__(self, path: str) -> dict:
        body = _http_get(
            self._t.local_kernel,
            path,
            headers={"X-API-Key": _require_key(self._key, path)},
            timeout=15.0,
        )
        try:
            return json.loads(body)
        except ValueError as e:
            raise ProbeUnavailable(
                f"kernel returned non-JSON for {path}: {body[:200]!r}"
            ) from e


class KernelJsonFetch:
    """ssh+curl counterpart of :class:`TunnelJsonFetch` for the no-setup
    fallback transport."""

    def __init__(self, api_key: Optional[str] = None):
        self._key = api_key if api_key is not None else _cfg("OPV_API_KEY", "")

    def __call__(self, path: str) -> dict:
        # `curl -s` exits 0 on a 403 and prints the error body, so the status
        # has to be carried out-of-band or it is simply lost (bug-500).
        raw = _run(
            f"curl.exe -s -m 8 -w '\\n%{{http_code}}' "
            f"-H 'X-API-Key: {_require_key(self._key, path)}' {_kernel_url(path)}",
            timeout=15,
        )
        body, status = _split_status(raw)
        if not 200 <= status < 300:
            raise ProbeUnavailable(
                f"kernel answered {status} for {path} "
                f"({'check OPV_API_KEY' if status in (401, 403) else 'not a state answer'}): "
                f"{body[:120].decode(errors='replace')!r}",
                status=status,
            )
        try:
            return json.loads(body)
        except ValueError as e:
            raise ProbeUnavailable(
                f"kernel returned non-JSON for {path}: {body[:200]!r}"
            ) from e


class TunnelHashSource:
    """Settle change-signal over the port-forward: GET /grabhash (agent ≥ v3)."""

    def __init__(self, tunnel: SshTunnel):
        self._t = tunnel

    def hash(self) -> str:
        body = _http_get(self._t.local_agent, "/grabhash").decode(errors="replace")
        try:
            return json.loads(body)["hash"]
        except (ValueError, KeyError) as e:
            raise RuntimeError(
                f"/grabhash unavailable (agent < v3?): {body[:120]}"
            ) from e


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
