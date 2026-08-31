"""Deployment backends — stand a ``clotocore`` daemon up somewhere and give
the harness an HTTP endpoint + admin key to drive, then tear it down.

* ``local``      — an isolated child process on this machine (phase 0/1)
* ``linux_vm``   — a Proxmox Linux guest via snapshot rollback (phase 2)
* ``windows_vm`` — a Proxmox Windows guest via snapshot rollback (phase 3)

All backends yield a :class:`RunningTarget` and honour the same
``start()`` / ``stop()`` contract so the harness is deployment-agnostic.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional


@dataclass
class RunningTarget:
    """A live daemon the harness can drive."""

    base_url: str
    api_key: str
    kind: str  # "local" | "linux_vm" | "windows_vm"
    pid: Optional[int] = None  # local only — for resource sampling
    stderr_path: Optional[str] = None  # local only — for log scraping
    db_path: Optional[str] = None  # for post-teardown integrity_check
    os_label: str = "unknown"  # e.g. "darwin", "linux", "windows"
