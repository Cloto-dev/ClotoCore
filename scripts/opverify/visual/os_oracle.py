"""OS-truth oracles for the apex — the third layer.

The dual oracle the apex started with asks two questions: *is it rendered*
(visual) and *did the kernel record it* (HTTP). An uninstall outcome is outside
both. The kernel is gone by then — that is the point of the step — and the
things that must be true afterwards are facts about the machine: the process
ended, the helper wrote its report, nothing of the product is left.

So these probes speak to the guest OS directly, over the same multiplexed SSH
the transports use. They satisfy :class:`~.interfaces.KernelProbe` (``check() ->
bool``) because that protocol is really "a deterministic assertion the visual
oracle is cross-checked against"; nothing in the driver requires it to be HTTP.

Every probe here is **read-only**. The sweep in particular is a detector, and a
detector that has only ever reported zero has not been shown to work — see
:class:`ResidueSweep` and the `expect` argument, which is why the purge journey
runs the sweep *before* the purge as well as after.

PowerShell is delivered as base64 UTF-16LE ``-EncodedCommand``: the guest's
default shell is PowerShell, so a script sent as a plain ssh command line is
parsed twice and mangles quoting (documented the hard way in
``VM_EXECUTOR_RUNBOOK.md``). Encoding removes that seam entirely.
"""

from __future__ import annotations

import base64
import json
import time
from typing import Iterable, List, Optional

from .backends_vm import _run

# Where an app the harness cares about lives on the VM. Overridable per probe;
# these are the defaults for the NSIS per-machine install.
INSTALL_PREFIX = r"C:\Program Files\ClotoCore"
PROCESS_NAME = "app"


def run_powershell(script: str, *, timeout: float = 60.0) -> str:
    """Run `script` on the guest and return its stdout as text."""
    encoded = base64.b64encode(script.encode("utf-16-le")).decode()
    out = _run(f"powershell -NoProfile -EncodedCommand {encoded}", timeout=timeout)
    return out.decode("utf-8", errors="replace")


def run_powershell_json(script: str, *, timeout: float = 60.0):
    """Run `script` and parse the single JSON document it prints.

    The guest's PowerShell wraps a remote session's output in a CLIXML preamble
    when anything reaches the error/progress streams, so the payload is fenced
    with markers rather than trusted to be the whole of stdout.
    """
    fenced = f"'<<<OPVJSON'\n{script}\n'OPVJSON>>>'"
    raw = run_powershell(fenced, timeout=timeout)
    try:
        body = raw.split("<<<OPVJSON", 1)[1].split("OPVJSON>>>", 1)[0]
    except IndexError:
        raise RuntimeError(f"probe produced no fenced payload: {raw[:300]!r}") from None
    return json.loads(body.strip())


class ProcessAbsent:
    """True once no process named `name` is running.

    Waits, because this is the assertion for bug-499: the kernel signals its
    shutdown and the shell exits *afterwards*, so sampling the instant the GUI
    stops repainting would race the exit it is meant to verify.
    """

    def __init__(self, name: str = PROCESS_NAME, *, wait_s: float = 30.0):
        self.name = name
        self.wait_s = wait_s
        self.detail = ""

    def count(self) -> int:
        out = run_powershell(
            f"(Get-Process {self.name} -EA SilentlyContinue | Measure-Object).Count"
        )
        for line in out.splitlines():
            line = line.strip()
            if line.isdigit():
                return int(line)
        raise RuntimeError(f"unparsable process count: {out[:200]!r}")

    def check(self) -> bool:
        deadline = time.monotonic() + self.wait_s
        while True:
            n = self.count()
            if n == 0:
                self.detail = "no process left"
                return True
            if time.monotonic() >= deadline:
                self.detail = f"{n} process(es) still running after {self.wait_s:.0f}s"
                return False
            time.sleep(2.0)


# Outcomes the executor writes per entry. "refused" and "failed" are the ones
# that mean the machine still holds something; "absent" means it was already
# gone, which is a clean outcome and not the same as "removed".
CLEAN_OUTCOMES = {"removed", "absent"}


class PurgeReportClean:
    """True once the detached helper has written a report and every entry in it
    landed on a clean outcome.

    The report is the helper's own account of what it did, and its *absence* is
    the exact signature bug-499 left behind: plan staged, helper copied, nothing
    executed, no report anywhere. So a missing report is a failure of this probe
    and not an inconclusive result.
    """

    def __init__(self, *, since: Optional[float] = None, wait_s: float = 120.0):
        # Only a report written after the journey started counts — the machine
        # may carry staging directories from earlier runs.
        self.since = since if since is not None else time.time()
        self.wait_s = wait_s
        self.detail = ""
        self.report: Optional[dict] = None

    def _find(self) -> Optional[dict]:
        epoch = int(self.since)
        script = f"""
$cut = [DateTimeOffset]::FromUnixTimeSeconds({epoch}).LocalDateTime
$r = Get-ChildItem $env:TEMP -Directory -EA SilentlyContinue |
  Where-Object {{ $_.Name -like 'clotocore-uninstall-*' }} |
  ForEach-Object {{ Get-ChildItem $_.FullName -Filter '*.report.json' -EA SilentlyContinue }} |
  Where-Object {{ $_.LastWriteTime -ge $cut }} |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($r) {{ Get-Content $r.FullName -Raw }} else {{ 'null' }}
"""
        return run_powershell_json(script)

    def check(self) -> bool:
        deadline = time.monotonic() + self.wait_s
        while True:
            report = self._find()
            if report:
                self.report = report
                entries = report.get("entries", [])
                bad = [e for e in entries if e.get("outcome") not in CLEAN_OUTCOMES]
                removed = sum(1 for e in entries if e.get("outcome") == "removed")
                if bad:
                    self.detail = (
                        f"{len(bad)} entr(y/ies) not clean: "
                        + ", ".join(f"{e.get('id')}={e.get('outcome')}" for e in bad[:5])
                    )
                    return False
                if removed == 0:
                    # Every entry "absent" means the helper ran against a machine
                    # that had nothing to remove — the fixture was empty, and a
                    # journey that passes on it has verified nothing.
                    self.detail = f"{len(entries)} entries, none removed (empty fixture?)"
                    return False
                self.detail = f"{removed} removed / {len(entries) - removed} absent"
                return True
            if time.monotonic() >= deadline:
                self.detail = f"no report written within {self.wait_s:.0f}s"
                return False
            time.sleep(3.0)


# The read-only sweep. Each item is (id, PowerShell test expression) and is
# reported when the expression is true — i.e. when the product is still there.
# Grown out of the incident sweep written on 2026-08-03; bug-497's residue (the
# vendor key alone, with the uninstall list already gone) is in here by name
# because a sweep that could not see it is what let that bug live.
def _sweep_script(install_prefix: str, data_dir: str, process_name: str) -> str:
    return f"""
$found = @()
if (Get-Process {process_name} -EA SilentlyContinue) {{ $found += 'process' }}
if (Test-Path '{install_prefix}') {{ $found += 'install_prefix' }}
if ('{data_dir}' -and (Test-Path '{data_dir}')) {{ $found += 'data_dir' }}
$arp = Get-ItemProperty HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*,`
  HKCU:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\* -EA SilentlyContinue |
  Where-Object {{ $_.DisplayName -like '*Cloto*' }}
if ($arp) {{ $found += 'arp_key' }}
# The product key must be gone. The manufacturer key *above* it is a different
# matter: `crates/core/src/defender/purge.rs` leaves it deliberately, because
# `cloto` is a vendor namespace and registry deletion is recursive — removing it
# would take a sibling product's keys with it. What is allowed to survive is an
# empty shell; a manufacturer key that still holds subkeys or values means the
# product key outlived the purge, which is bug-497's shape.
foreach ($h in @('HKLM:\\Software\\cloto', 'HKCU:\\Software\\cloto')) {{
  if (Test-Path $h) {{
    $k = Get-Item $h
    if ($k.SubKeyCount -gt 0 -or $k.ValueCount -gt 0) {{ $found += 'vendor_key_not_empty' }}
  }}
}}
$run = Get-ItemProperty HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run -EA SilentlyContinue
if ($run -and ($run.PSObject.Properties | Where-Object {{ $_.Value -like '*ClotoCore*' }})) {{ $found += 'autorun' }}
$startup = Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs\\Startup'
if (Get-ChildItem $startup -Filter '*Cloto*' -EA SilentlyContinue) {{ $found += 'startup_shortcut' }}
if (Get-ScheduledTask -EA SilentlyContinue | Where-Object {{ $_.TaskName -like '*Cloto*' }}) {{ $found += 'scheduled_task' }}
foreach ($d in @(
  (Join-Path $env:ProgramData 'Microsoft\\Windows\\Start Menu\\Programs'),
  'C:\\Users\\Public\\Desktop',
  (Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs')
)) {{
  if (Get-ChildItem $d -Filter '*Cloto*' -EA SilentlyContinue) {{ $found += 'shortcut' }}
}}
ConvertTo-Json -Compress @($found | Sort-Object -Unique)
"""


class ResidueSweep:
    """Sweep the machine for anything the product left behind.

    ``expect="empty"`` is the post-purge assertion; ``expect="present"`` is the
    pre-purge one, and it is not ceremony: a sweep that has never reported a
    non-empty result is indistinguishable from a sweep that is broken. Running
    both ends in the same journey means the zero at the end was produced by a
    detector observed working minutes earlier, on the same machine.
    """

    def __init__(
        self,
        *,
        data_dir: str = "",
        install_prefix: str = INSTALL_PREFIX,
        process_name: str = PROCESS_NAME,
        expect: str = "empty",
        ignore: Iterable[str] = (),
    ):
        if expect not in ("empty", "present"):
            raise ValueError(f"expect must be 'empty' or 'present', got {expect!r}")
        self.data_dir = data_dir
        self.install_prefix = install_prefix
        self.process_name = process_name
        self.expect = expect
        self.ignore = set(ignore)
        self.detail = ""
        self.found: List[str] = []

    def sweep(self) -> List[str]:
        found = run_powershell_json(
            _sweep_script(self.install_prefix, self.data_dir, self.process_name)
        )
        if found is None:
            found = []
        if isinstance(found, str):  # ConvertTo-Json collapses a 1-element array
            found = [found]
        return [f for f in found if f not in self.ignore]

    def check(self) -> bool:
        self.found = self.sweep()
        self.detail = ", ".join(self.found) if self.found else "nothing found"
        if self.expect == "present":
            return bool(self.found)
        return not self.found
