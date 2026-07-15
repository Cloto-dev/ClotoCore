"""Reproducibly deploy the session-1 actuator agent onto the Windows VM (#238).

The visual apex needs :mod:`.vm_agent` (``opv_agent.py``) running inside the
interactive Windows desktop (session 1) so mss can screenshot the real screen
and pyautogui can inject OS input. Until now that agent was placed on the VM by
hand — manual scp into ``C:\\opv`` plus a hand-rolled Task Scheduler
registration — and the recipe lived only in a scratchpad shell script and
episodic memory. When the scratchpad was cleared the deploy mechanism was lost;
the committed harness (``vm_agent.py``) survived but *how to stand it up* did
not. This module is that recipe, committed and idempotent, so a pristine
snapshot re-take or a brand-new VM comes up in one shot::

    python -m scripts.opverify.visual.deploy_agent            # full deploy
    python -m scripts.opverify.visual.deploy_agent --redeploy # re-copy + restart only
    python -m scripts.opverify.visual.deploy_agent --status   # health probe, no changes

The deploy is **verified-not-speculative**: after standing the agent up it polls
``/health`` and asserts the reported protocol ``version`` equals the one declared
in the committed ``vm_agent.py`` source — proving the running agent *is* the
committed code, not a stale copy left on the VM.

Reproduced end state (the "correct form", captured from VM104 2026-07-15):
  * ``C:\\opv\\opv_agent.py`` = this repo's ``vm_agent.py``
  * deps ``mss`` + ``pyautogui`` importable by the VM's Python
  * Task Scheduler task ``opv_agent``: Action ``pythonw.exe C:\\opv\\opv_agent.py``,
    Principal ``LogonType=Interactive RunLevel=Highest`` (session-1 desktop),
    Trigger ``AtLogOn`` (survives reboot), no execution time limit
  * agent answering ``127.0.0.1:8900/health`` with the committed protocol version

Config is the same env-driven, secret-free contract the backends use
(:func:`.backends_vm._cfg`): ``OPV_VM_USER`` / ``OPV_VM_IP`` / ``OPV_AGENT_PORT``
/ ``OPV_SSH_PERSIST`` / ``OPV_SSH_CONTROL_PATH``.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import time
from typing import Optional, Tuple

from .backends_vm import _cfg

AGENT_SRC = os.path.join(os.path.dirname(__file__), "vm_agent.py")
REMOTE_DIR = r"C:\opv"
REMOTE_AGENT = r"C:\opv\opv_agent.py"
REMOTE_AGENT_SCP = "C:/opv/opv_agent.py"  # scp wants forward slashes
TASK_NAME = "opv_agent"


def _vm() -> str:
    return f"{_cfg('OPV_VM_USER', 'PC')}@{_cfg('OPV_VM_IP', '192.0.2.252')}"


def _ssh_base() -> list:
    """SSH prefix with connection multiplexing so the deploy's many small
    round trips reuse one authenticated master (same contract as the backends)."""
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
    ]


def _ssh(ps: str, timeout: float = 60.0) -> str:
    """Run one PowerShell command on the VM (its default shell) and return
    stdout. subprocess list form means no local shell — the PowerShell string
    may contain any quotes; only PowerShell parses it. Raises on non-zero exit."""
    proc = subprocess.run(
        _ssh_base() + [_vm(), ps], capture_output=True, timeout=timeout
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"VM ssh failed ({proc.returncode}): "
            f"{proc.stderr.decode(errors='replace')[:400]}"
        )
    return proc.stdout.decode(errors="replace")


def _scp(local: str, remote_fwd: str, timeout: float = 60.0) -> None:
    cmd = [
        "scp",
        "-o",
        "ConnectTimeout=8",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        f"ControlPath={_cfg('OPV_SSH_CONTROL_PATH', '/tmp/opv-ssh-%r@%h:%p')}",
        local,
        f"{_vm()}:{remote_fwd}",
    ]
    proc = subprocess.run(cmd, capture_output=True, timeout=timeout)
    if proc.returncode != 0:
        raise RuntimeError(
            f"scp failed ({proc.returncode}): "
            f"{proc.stderr.decode(errors='replace')[:400]}"
        )


def _expected_version() -> int:
    """The agent protocol version declared in the committed source — the anchor
    the post-deploy health check asserts against."""
    with open(AGENT_SRC, encoding="utf-8") as fh:
        m = re.search(r'"version":\s*(\d+)', fh.read())
    if not m:
        raise RuntimeError(f"no protocol version found in {AGENT_SRC}")
    return int(m.group(1))


def _resolve_python() -> Tuple[Optional[str], Optional[str]]:
    """Return (python.exe, pythonw.exe) full paths on the VM, or (None, None) if
    Python is absent. Uses Get-Command first (PATH), then globs the standard
    install roots (a non-interactive ssh session may not see a freshly installed
    Python on PATH)."""
    out = _ssh(
        "$p=(Get-Command python -EA SilentlyContinue).Source; "
        "$w=(Get-Command pythonw -EA SilentlyContinue).Source; "
        "if(-not $p){ $c=@(); "
        "$c+=Get-ChildItem 'C:\\Program Files\\Python3*\\python.exe' -EA SilentlyContinue; "
        '$c+=Get-ChildItem "$env:LOCALAPPDATA\\Programs\\Python\\Python3*\\python.exe" -EA SilentlyContinue; '
        "if($c){ $p=$c[0].FullName } }; "
        "if($p -and -not $w){ $w=$p -replace 'python\\.exe$','pythonw.exe' }; "
        "Write-Output ($p + '|' + $w)"
    )
    py, _, pyw = out.strip().partition("|")
    return (py or None), (pyw or None)


def _ensure_python() -> Tuple[str, str]:
    py, pyw = _resolve_python()
    if py:
        return py, pyw
    print("  python not found — installing Python.3.12 via winget (may take minutes)")
    _ssh(
        "winget install -e --id Python.Python.3.12 "
        "--accept-package-agreements --accept-source-agreements --silent",
        timeout=900,
    )
    py, pyw = _resolve_python()
    if not py:
        raise RuntimeError(
            "Python still not resolvable after winget install — install it "
            "manually on the VM and re-run"
        )
    return py, pyw


def _ensure_dir() -> None:
    _ssh(f"New-Item -ItemType Directory -Force -Path '{REMOTE_DIR}' | Out-Null")


def _ensure_deps(python_exe: str) -> None:
    _ssh(
        f"& '{python_exe}' -m pip install --quiet --disable-pip-version-check "
        "mss pyautogui",
        timeout=600,
    )


def _register_task(pythonw_exe: str) -> None:
    """Register (or replace, -Force) the interactive session-1 task that keeps
    the agent alive across logon. UserId = the current interactive user, so a
    new VM with a different console account still works."""
    ps = (
        f"$a=New-ScheduledTaskAction -Execute '{pythonw_exe}' -Argument '{REMOTE_AGENT}'; "
        "$t=New-ScheduledTaskTrigger -AtLogOn; "
        "$p=New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Highest; "
        "$s=New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries "
        "-DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero); "
        f"Register-ScheduledTask -TaskName '{TASK_NAME}' -Action $a -Trigger $t "
        "-Principal $p -Settings $s -Force | Out-Null"
    )
    _ssh(ps)


def _restart_agent() -> None:
    """Stop whatever holds the agent port, then (re)start via the task so the
    freshly copied file is the one that loads."""
    _ssh(
        f"Stop-ScheduledTask -TaskName '{TASK_NAME}' -EA SilentlyContinue; "
        "try{ (Get-NetTCPConnection -LocalPort "
        f"{_cfg('OPV_AGENT_PORT', '8900')} -State Listen -EA Stop).OwningProcess | "
        "ForEach-Object{ Stop-Process -Id $_ -Force -EA SilentlyContinue } }catch{}; "
        "Start-Sleep -Milliseconds 300; "
        f"Start-ScheduledTask -TaskName '{TASK_NAME}'"
    )


def _health(timeout: float = 10.0) -> dict:
    out = _ssh(
        f"curl.exe -s http://127.0.0.1:{_cfg('OPV_AGENT_PORT', '8900')}/health",
        timeout=timeout,
    )
    return json.loads(out)


def _wait_health(expected_version: int, tries: int = 20, delay: float = 0.5) -> dict:
    last = None
    for _ in range(tries):
        try:
            h = _health()
            if h.get("ok") and h.get("version") == expected_version:
                return h
            last = h
        except Exception as e:  # not up yet / partial read
            last = repr(e)
        time.sleep(delay)
    raise RuntimeError(
        f"agent did not report healthy version={expected_version} in time "
        f"(last={last!r})"
    )


def status() -> int:
    want = _expected_version()
    try:
        h = _health()
    except Exception as e:
        print(f"agent NOT reachable on {_vm()}:{_cfg('OPV_AGENT_PORT', '8900')}: {e}")
        return 1
    ok = h.get("ok") and h.get("version") == want
    print(json.dumps(h))
    print(
        f"protocol version: running={h.get('version')} expected={want} "
        f"-> {'MATCH' if ok else 'DRIFT'}"
    )
    return 0 if ok else 1


def deploy(redeploy: bool = False) -> int:
    want = _expected_version()
    vm = _vm()
    print(f"deploy opv_agent -> {vm}  (expected protocol version {want})")

    print("  preflight: ssh reachable")
    _ssh("Write-Output ok")

    if redeploy:
        _, pyw = _resolve_python()
        if not pyw:
            print("  redeploy: Python not resolvable — falling back to full deploy")
            return deploy(redeploy=False)
    else:
        print("  ensure C:\\opv")
        _ensure_dir()
        print("  ensure Python + deps (mss, pyautogui)")
        py, pyw = _ensure_python()
        _ensure_deps(py)

    print(f"  copy vm_agent.py -> {REMOTE_AGENT}")
    _ensure_dir()  # cheap + idempotent; guarantees the scp target exists
    _scp(AGENT_SRC, REMOTE_AGENT_SCP)

    if not redeploy:
        print(
            "  register Task Scheduler task (Interactive / RunLevel Highest / AtLogOn)"
        )
        _register_task(pyw)

    print("  (re)start agent")
    _restart_agent()

    print("  verify /health")
    h = _wait_health(want)
    print(f"  OK: {json.dumps(h)}")
    return 0


def main(argv) -> int:
    if "--status" in argv:
        return status()
    return deploy(redeploy="--redeploy" in argv)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
