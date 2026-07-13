"""Local deployment backend — boot an isolated ``clotocore`` child process
on this machine and drive it over loopback HTTP (phase 0 / 1).

Isolation model: the throwaway SQLite DB (``DATABASE_URL``) and MCP sandbox
(``CLOTO_SANDBOX_DIR``) are redirected into a fresh temp dir, which is what
matters for the state-corruption oracles. Note the kernel's ``data_dir()``
is anchored to the binary location (no env override), so log files / the
MCP venv still resolve next to the binary in a dev checkout — acceptable for
the local tier, where fast catalog iteration is the goal; true per-run
isolation is provided by the VM tiers' pristine snapshots.

Teardown is via the authenticated ``POST /api/system/shutdown`` route (the
kernel installs no SIGTERM handler), with a SIGKILL fallback.
"""

from __future__ import annotations

import os
import secrets
import shutil
import signal
import socket
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Optional

from ..client import ClotoClient
from . import RunningTarget


def _repo_root() -> Path:
    # scripts/opverify/deploy/local.py -> parents[3] == repo root
    return Path(__file__).resolve().parents[3]


def _default_binary() -> Optional[Path]:
    root = _repo_root()
    for rel in ("target/debug/clotocore", "target/release/clotocore"):
        p = root / rel
        if p.exists():
            return p
    return None


def _free_port() -> int:
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]
    finally:
        s.close()


class LocalDeployment:
    """Start/stop an isolated local kernel daemon."""

    kind = "local"

    def __init__(
        self,
        binary: Optional[str] = None,
        port: Optional[int] = None,
        keep_dir: bool = False,
        env_overrides: Optional[dict] = None,
    ):
        self.binary = Path(binary) if binary else _default_binary()
        self.port = port
        self.keep_dir = keep_dir
        self.env_overrides = env_overrides or {}
        self._proc: Optional[subprocess.Popen] = None
        self._dir: Optional[str] = None
        self._stderr_fh = None
        self.target: Optional[RunningTarget] = None

    def start(self) -> RunningTarget:
        if self.binary is None or not self.binary.exists():
            raise FileNotFoundError(
                "clotocore binary not found; build it with "
                "`cargo build --bin clotocore` or pass --binary"
            )
        self._dir = tempfile.mkdtemp(prefix="opverify-local.")
        db_path = os.path.join(self._dir, "cloto.db")
        sandbox = os.path.join(self._dir, "sandbox")
        stderr_path = os.path.join(self._dir, "stderr.log")
        key = secrets.token_hex(32)
        port = self.port or _free_port()

        env = dict(os.environ)
        env.pop("CLOTO_DEBUG_SKIP_AUTH", None)  # force real auth
        env.update(
            {
                "CLOTO_API_KEY": key,
                "PORT": str(port),
                "BIND_ADDRESS": "127.0.0.1",
                "DATABASE_URL": f"sqlite:{db_path}",
                "CLOTO_SANDBOX_DIR": sandbox,
                "RUST_LOG": env.get("RUST_LOG", "info"),
            }
        )
        env.update(self.env_overrides)

        self._stderr_fh = open(stderr_path, "wb")
        self._proc = subprocess.Popen(
            [str(self.binary)],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=self._stderr_fh,
            cwd=self._dir,
        )
        self.target = RunningTarget(
            base_url=f"http://127.0.0.1:{port}",
            api_key=key,
            kind=self.kind,
            pid=self._proc.pid,
            stderr_path=stderr_path,
            db_path=db_path,
            os_label=os.uname().sysname.lower() if hasattr(os, "uname") else "unknown",
        )
        return self.target

    def wait_ready(self, timeout: float = 60.0) -> None:
        assert self.target is not None
        client = ClotoClient(self.target.base_url, self.target.api_key)
        # fail fast if the process dies during boot
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self._proc and self._proc.poll() is not None:
                raise RuntimeError(
                    f"clotocore exited during boot (code {self._proc.returncode}); "
                    f"see {self.target.stderr_path}"
                )
            try:
                client.wait_healthy(timeout=2.0, interval=0.4)
                return
            except TimeoutError:
                continue
        raise TimeoutError(f"kernel not ready within {timeout}s")

    def stop(self, shutdown_timeout: float = 25.0) -> None:
        if not self._proc:
            return
        if self.target:
            try:
                ClotoClient(self.target.base_url, self.target.api_key).post(
                    "/api/system/shutdown", timeout=10.0
                )
            except Exception:
                pass  # fall through to poll + SIGKILL
        deadline = time.monotonic() + shutdown_timeout
        while time.monotonic() < deadline:
            if self._proc.poll() is not None:
                break
            time.sleep(0.3)
        if self._proc.poll() is None:
            try:
                self._proc.send_signal(signal.SIGKILL)
            except OSError:
                pass
            try:
                self._proc.wait(timeout=5.0)
            except subprocess.TimeoutExpired:
                pass
        if self._stderr_fh:
            self._stderr_fh.close()
            self._stderr_fh = None

    def cleanup(self) -> None:
        """Remove the throwaway run dir. Call AFTER post-teardown oracles
        (corruption_check / final log scrape) have read from it. Scoped to
        our own ``opverify-local.*`` prefix so it can never touch anything
        else."""
        if (
            not self.keep_dir
            and self._dir
            and os.path.basename(self._dir).startswith("opverify-local.")
        ):
            shutil.rmtree(self._dir, ignore_errors=True)
            self._dir = None

    @property
    def exited_cleanly(self) -> Optional[bool]:
        if not self._proc:
            return None
        rc = self._proc.poll()
        return None if rc is None else rc == 0
