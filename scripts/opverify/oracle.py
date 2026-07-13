"""Global oracles — the machinery that decides whether the *daemon as a
whole* stayed healthy while the operation catalog ran.

Per-operation success is asserted inside each catalog operation
(``assert_success``). These oracles run *between* operations and at the end
of a run to catch cross-cutting failures that no single operation asserts:

* **liveness**   — the process is alive and ``/api/system/health`` == ok
* **integrity**  — ``/api/health/scan`` stays Healthy (referential + audit chain)
* **resource**   — RSS / open-FD / child-process count do not grow without
                   bound and return to baseline after teardown (MCP orphans)
* **log**        — captured stderr contains no panic / ERROR lines
* **corruption** — ``PRAGMA integrity_check`` on the throwaway DB (run after
                   teardown, since WAL is live while the daemon runs)

Everything is stdlib + a couple of POSIX CLIs (``ps``, ``pgrep``,
``sqlite3``); on platforms/tools that are unavailable a sample degrades to
``None`` rather than failing the run.
"""

from __future__ import annotations

import re
import subprocess
from dataclasses import dataclass, field
from typing import Optional

from .client import ApiError, ClotoClient

# stderr lines that indicate a real fault (case-insensitive).
_PANIC_PATTERNS = [
    re.compile(r"thread '.*' panicked", re.I),
    re.compile(r"\bpanicked at\b", re.I),
    re.compile(r"\bERROR\b"),
    re.compile(r"\bfatal\b", re.I),
]


@dataclass
class ResourceSample:
    """A point-in-time snapshot of the daemon's resource footprint."""

    rss_kb: Optional[int] = None
    open_fds: Optional[int] = None
    child_count: Optional[int] = None

    def as_dict(self) -> dict:
        return {
            "rss_kb": self.rss_kb,
            "open_fds": self.open_fds,
            "child_count": self.child_count,
        }


def _run(cmd: list[str], timeout: float = 5.0) -> Optional[str]:
    try:
        out = subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout
        )
        if out.returncode != 0:
            return None
        return out.stdout
    except (OSError, subprocess.SubprocessError):
        return None


def _descendant_pids(pid: int) -> list[int]:
    """Recursively collect child pids via ``pgrep -P`` (POSIX)."""
    found: list[int] = []
    frontier = [pid]
    seen = {pid}
    while frontier:
        parent = frontier.pop()
        out = _run(["pgrep", "-P", str(parent)])
        if not out:
            continue
        for tok in out.split():
            try:
                child = int(tok)
            except ValueError:
                continue
            if child not in seen:
                seen.add(child)
                found.append(child)
                frontier.append(child)
    return found


def sample_resources(pid: int) -> ResourceSample:
    """Sample RSS + open FD count + descendant process count for ``pid``.

    Best-effort: any metric that cannot be read on this platform is ``None``.
    """
    s = ResourceSample()
    rss = _run(["ps", "-o", "rss=", "-p", str(pid)])
    if rss and rss.strip().isdigit():
        s.rss_kb = int(rss.strip())
    # open fds: prefer /proc (Linux); fall back to lsof (macOS)
    try:
        with open(f"/proc/{pid}/fd") as _:  # noqa: SIM115 - existence probe
            pass
    except OSError:
        pass
    proc_fd = _run(["bash", "-c", f"ls /proc/{pid}/fd 2>/dev/null | wc -l"])
    if proc_fd and proc_fd.strip().isdigit() and int(proc_fd.strip()) > 0:
        s.open_fds = int(proc_fd.strip())
    else:
        lsof = _run(["bash", "-c", f"lsof -p {pid} 2>/dev/null | wc -l"])
        if lsof and lsof.strip().isdigit():
            s.open_fds = int(lsof.strip())
    s.child_count = len(_descendant_pids(pid))
    return s


@dataclass
class OracleReport:
    """Accumulated cross-cutting findings for a whole run."""

    liveness_ok: bool = True
    integrity_ok: bool = True
    log_clean: bool = True
    corruption_ok: Optional[bool] = None
    findings: list[str] = field(default_factory=list)
    baseline: Optional[ResourceSample] = None
    final: Optional[ResourceSample] = None
    _log_offset: int = 0

    def note(self, msg: str) -> None:
        self.findings.append(msg)

    @property
    def ok(self) -> bool:
        return (
            self.liveness_ok
            and self.integrity_ok
            and self.log_clean
            and self.corruption_ok is not False
        )

    def as_dict(self) -> dict:
        return {
            "ok": self.ok,
            "liveness_ok": self.liveness_ok,
            "integrity_ok": self.integrity_ok,
            "log_clean": self.log_clean,
            "corruption_ok": self.corruption_ok,
            "baseline": self.baseline.as_dict() if self.baseline else None,
            "final": self.final.as_dict() if self.final else None,
            "findings": self.findings,
        }


def check_liveness(client: ClotoClient, report: OracleReport) -> None:
    try:
        data = client.get("/api/system/health", auth=False, timeout=5.0)
        if not (isinstance(data, dict) and data.get("status") == "ok"):
            report.liveness_ok = False
            report.note(f"liveness: unexpected health body {data!r}")
    except (ApiError, OSError) as e:
        report.liveness_ok = False
        report.note(f"liveness: health unreachable ({e})")


def check_integrity(client: ClotoClient, report: OracleReport) -> None:
    """Run the kernel's deep health scan and flag non-Healthy status.

    The scan route (``/api/health/scan``) and its status enum are confirmed
    against the running instance in :mod:`opverify.catalog.health`; here we
    only care that ``status`` is the healthy value.
    """
    try:
        data = client.get("/api/health/scan", params={"fresh": "true"}, timeout=30.0)
    except ApiError as e:
        report.integrity_ok = False
        report.note(f"integrity: health scan failed ({e})")
        return
    status = None
    if isinstance(data, dict):
        status = str(data.get("status", "")).lower()
    if status and status not in ("healthy", "ok"):
        report.integrity_ok = False
        report.note(f"integrity: health scan status={status}")


def scrape_log(log_path: Optional[str], report: OracleReport) -> None:
    """Scan newly-appended stderr for panic/ERROR lines since last offset."""
    if not log_path:
        return
    try:
        with open(log_path, "r", encoding="utf-8", errors="replace") as f:
            f.seek(report._log_offset)
            chunk = f.read()
            report._log_offset = f.tell()
    except OSError:
        return
    for line in chunk.splitlines():
        for pat in _PANIC_PATTERNS:
            if pat.search(line):
                report.log_clean = False
                report.note(f"log: {line.strip()[:200]}")
                break


def check_corruption(db_path: Optional[str], report: OracleReport) -> None:
    """Run ``PRAGMA integrity_check`` via the sqlite3 CLI (post-teardown)."""
    if not db_path:
        return
    out = _run(["sqlite3", db_path, "PRAGMA integrity_check;"], timeout=30.0)
    if out is None:
        report.corruption_ok = None  # sqlite3 unavailable — cannot judge
        return
    report.corruption_ok = out.strip() == "ok"
    if not report.corruption_ok:
        report.note(f"corruption: integrity_check -> {out.strip()[:200]}")
