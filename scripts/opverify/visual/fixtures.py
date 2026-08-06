"""Start-state fixtures — what has to be true before step one.

A journey is written against a machine in a particular state. `chat-render`
needs an engine, a provider key and a transcript taller than the pane;
`danger-zone-purge` needs an install with data to remove. `clean-install-*` is
deliberately data-free, so every one of those preconditions is built by hand,
and on 2026-08-03 building them cost about thirty minutes. A rollback is
sixteen seconds. The asymmetry is the whole reason this module exists: state a
journey can declare is state that can be snapshotted once and resumed forever.

The rule the fixtures enforce is the one the harness learned the expensive way:
**an unmet precondition fails the run, it does not pass it.** A chat journey on
an empty thread asserts that a reply is visible without scrolling in a pane
nothing has ever filled — it passes while testing nothing. A purge journey on a
data-free install builds an empty plan and reports that everything it did not
remove is absent. Both were observed (2026-08-03, 2026-08-05); both are green.

Two deliberate limits:

* Verification runs against the **running kernel**, so it asserts the data
  state, not the disk. `KernelResponds` is what fails, by name and with the
  launch recipe, when the app is not up yet.
* Rollback is **never automatic**. `qm rollback` discards the live state and
  there is no snapshot of "now" to come back to; on 2026-08-03 it threw away
  the only configured environment in existence. So the default is to check and
  fail with the remedy printed, and rolling the VM is an explicit flag that
  names what it is about to discard.
"""

from __future__ import annotations

import os
import subprocess
import time
from dataclasses import dataclass, field
from typing import Callable, Dict, List, Optional, Tuple

# (ok, human-readable detail) — the same shape every check returns, so the
# report can print a failed one without knowing what it asked.
Verdict = Tuple[bool, str]

# The console store the chat pane actually renders. Messages posted to
# /api/chat land in kernel history and NOT here, which is why a thread built
# through the API leaves the pane empty (VM_EXECUTOR_RUNBOOK.md).
CHAT_STORE = "/api/chat/{agent_id}/messages"

DEFAULT_AGENT = "agent.cloto_default"


def _unwrap(body: dict) -> dict:
    """Every kernel handler wraps its payload in a ``{"data": ...}`` envelope
    (handlers/response.rs ``ok_data``). Checking the envelope for the payload's
    keys is a real bug this codebase has already shipped once (2026-07-31)."""
    return body.get("data", body) if isinstance(body, dict) else body


# --------------------------------------------------------------------------
# Checks — each one is a deterministic question about the kernel's state
# --------------------------------------------------------------------------
@dataclass
class KernelResponds:
    """The app is running and its admin API answers.

    First in every fixture: when it fails, the others would fail too, and
    their messages would blame the state instead of the missing process.
    """

    name: str = "kernel-responds"

    def check(self, fetch_json) -> Verdict:
        try:
            body = _unwrap(fetch_json("/api/system/health"))
        except Exception as e:  # transport, auth, non-JSON — all mean "no"
            return False, f"no answer from the kernel admin API ({e})"
        status = body.get("status")
        return status == "ok", f"health status={status!r}"


@dataclass
class SetupComplete:
    """Onboarding is (or is not) behind us.

    A fresh install comes up on the onboarding carousel, and a journey written
    against the main window then fails eight targets in a row — read once as
    nine product defects (2026-08-05). Asking here turns that into one failure
    with the right name.
    """

    expect: bool = True
    name: str = "setup-complete"

    def check(self, fetch_json) -> Verdict:
        body = _unwrap(fetch_json("/api/setup/status"))
        got = bool(body.get("setup_complete"))
        return got == self.expect, f"setup_complete={got} (want {self.expect})"


@dataclass
class PurgePlanEntries:
    """The uninstall plan at `tier` has real work in it.

    `PurgeReportClean` refuses a report whose entries are all `absent`, so a
    purge journey on a data-free install cannot pass — but it discovers that
    at the end, after uninstalling the app. Asking up front costs one GET.
    """

    tier: int = 4
    minimum: int = 1
    name: str = "purge-plan-has-entries"

    def check(self, fetch_json) -> Verdict:
        body = _unwrap(fetch_json(f"/api/system/uninstall/plan?tier={self.tier}"))
        summary = body.get("summary") or {}
        entries = summary.get("entries", 0)
        return entries >= self.minimum, (
            f"tier-{self.tier} plan has {entries} entries "
            f"(want >= {self.minimum}), {summary.get('total_bytes', 0)} bytes"
        )


@dataclass
class ProviderReady:
    """A named LLM provider has a key and its engine is connected.

    Fields are the ones `list_llm_providers` actually serializes: `has_key`
    (the key is masked — only the boolean ever leaves the kernel), `configured`
    and `engine_status` ∈ {connected, disconnected, uninstalled, catalog_only}.
    """

    provider_id: str
    require_connected: bool = True
    name: str = ""

    def __post_init__(self):
        self.name = self.name or f"provider-ready:{self.provider_id}"

    def check(self, fetch_json) -> Verdict:
        body = _unwrap(fetch_json("/api/llm/providers"))
        providers = body.get("providers", [])
        for p in providers:
            if p.get("id") != self.provider_id:
                continue
            status = p.get("engine_status")
            ok = bool(p.get("has_key")) and bool(p.get("configured"))
            if self.require_connected:
                ok = ok and status == "connected"
            return ok, (
                f"has_key={bool(p.get('has_key'))} "
                f"configured={bool(p.get('configured'))} engine_status={status!r}"
            )
        listed = ", ".join(str(p.get("id")) for p in providers) or "none"
        return False, f"provider {self.provider_id!r} is not present (providers: {listed})"


@dataclass
class TranscriptClean:
    """No turn in the transcript is an engine error.

    Depth alone is not enough, and this is not hypothetical: building this
    fixture on 2026-08-06 hit the provider's rate limit and two of the nine
    turns came back as `[Error] Rate limit exceeded …`, which the store
    persists as an ordinary agent row. The row count went up, the check that
    only counted rows was satisfied, and the pane was two thirds full of
    yellow error boxes — a start state that gives a visual assessor defects to
    report that have nothing to do with the journey.

    `[Error]` is the product's own marker: the console renders the error style
    on exactly this prefix (`AgentConsole.tsx`).
    """

    agent_id: str = DEFAULT_AGENT
    name: str = "transcript-clean"

    def check(self, fetch_json) -> Verdict:
        body = _unwrap(fetch_json(CHAT_STORE.format(agent_id=self.agent_id)))
        bad = [m for m in body.get("messages", []) if "[Error]" in str(m.get("content", ""))]
        return not bad, (
            "no engine-error turns" if not bad else f"{len(bad)} turns are engine errors"
        )


@dataclass
class ThreadDepth:
    """The console store holds at least `minimum` messages.

    This is the precondition `reply-rendered-without-scrolling` is meaningless
    without: on a transcript shorter than the viewport every new turn is
    visible however the scroll logic behaves, so the assertion passes on a
    broken pane (the bug-498 class the journey exists to catch).
    """

    minimum: int
    agent_id: str = DEFAULT_AGENT
    name: str = "thread-depth"

    def check(self, fetch_json) -> Verdict:
        body = _unwrap(fetch_json(CHAT_STORE.format(agent_id=self.agent_id)))
        messages = body.get("messages", [])
        return len(messages) >= self.minimum, (
            f"{len(messages)} messages in the console store "
            f"(want >= {self.minimum})"
        )


# --------------------------------------------------------------------------
# The lineage
# --------------------------------------------------------------------------
@dataclass
class Fixture:
    """A named start state: how to recognize it, and how to get back to it.

    `snapshot` is the VM 104 snapshot that restores it (None = it has to be
    built by hand, and `build` says how). Snapshot names carry the app version
    dash-encoded because Proxmox forbids dots, and they go stale on a version
    bump by design — the ritual for re-cutting one is in
    VM_EXECUTOR_RUNBOOK.md, "VM state reset".
    """

    name: str
    summary: str
    checks: List[object]
    snapshot: Optional[str] = None
    build: str = ""


FIXTURES: Dict[str, Fixture] = {
    "first-run": Fixture(
        name="first-run",
        summary="installed and never launched — the app comes up on the onboarding carousel",
        snapshot="clean-install-0-6-8-beta-2",
        checks=[KernelResponds(), SetupComplete(expect=False)],
        build=(
            "install the NSIS package over `agent-base-20260727` and do not "
            "launch it; the journey's own first step is the launch."
        ),
    ),
    "onboarded": Fixture(
        name="onboarded",
        summary=(
            "installed, onboarding completed, main window — and %APPDATA% data "
            "that a purge plan can actually enumerate"
        ),
        snapshot="onboarded-0-6-8-beta-4",
        checks=[
            KernelResponds(),
            SetupComplete(expect=True),
            PurgePlanEntries(tier=4, minimum=1),
        ],
        build=(
            "from `first-run`: launch with a known CLOTO_API_KEY, walk onboarding "
            "(name -> SKIP the assistant preset -> tick the admin-key "
            "acknowledgement, and do NOT press regenerate)."
        ),
    ),
    "configured-chat": Fixture(
        name="configured-chat",
        summary=(
            "onboarded, plus a connected engine with a key and a transcript "
            "taller than the chat pane"
        ),
        snapshot="configured-chat-0-6-8-beta-4",
        checks=[
            KernelResponds(),
            SetupComplete(expect=True),
            ProviderReady("cerebras"),
            ThreadDepth(minimum=18),
            TranscriptClean(),
        ],
        build=(
            "from `onboarded`: POST /api/marketplace/install {server_id: cerebras, "
            "auto_start: true} -> POST /api/llm/providers/cerebras/key -> post the "
            "filler turns to /api/chat AS THE PANE'S IDENTITY "
            "(source {type: User, id: 'default'}): the kernel persists both sides "
            "under user_id = source.id, and the pane reads user_id 'default'. "
            "Pace them — the provider's rate limit turns a turn into an "
            "`[Error]` row that still counts as depth. Snapshot with the chat "
            "view open: the journey's first step expects it."
        ),
    ),
}


# --------------------------------------------------------------------------
# Verification
# --------------------------------------------------------------------------
@dataclass
class FixtureReport:
    fixture: str
    results: List[Tuple[str, bool, str]] = field(default_factory=list)
    # Set when a check could not be evaluated at all (transport down, 403).
    error: str = ""

    @property
    def ok(self) -> bool:
        return not self.error and all(ok for _, ok, _ in self.results)

    def failures(self) -> List[Tuple[str, bool, str]]:
        return [r for r in self.results if not r[1]]

    def as_dict(self) -> dict:
        return {
            "fixture": self.fixture,
            "ok": self.ok,
            "error": self.error,
            "checks": [
                {"name": n, "ok": ok, "detail": d} for n, ok, d in self.results
            ],
        }


def verify(name: str, fetch_json) -> FixtureReport:
    """Evaluate every check of `name` against the live kernel.

    Every check runs even after one fails: "no engine AND an empty thread" is
    one trip to the VM to fix, and stopping at the first failure would make it
    two.
    """
    fixture = FIXTURES.get(name)
    if fixture is None:
        known = ", ".join(sorted(FIXTURES))
        return FixtureReport(fixture=name, error=f"unknown fixture (known: {known})")
    report = FixtureReport(fixture=name)
    for check in fixture.checks:
        try:
            ok, detail = check.check(fetch_json)
        except Exception as e:
            ok, detail = False, f"check raised {type(e).__name__}: {e}"
        report.results.append((getattr(check, "name", type(check).__name__), ok, detail))
    return report


def remedy(name: str) -> str:
    """What to do about an unsatisfied fixture, printed where it failed."""
    fixture = FIXTURES.get(name)
    if fixture is None:
        return ""
    lines = [f"fixture {fixture.name!r}: {fixture.summary}"]
    if fixture.snapshot:
        lines.append(
            f"  restore it:  python -m scripts.opverify.visual.run_vm <journey> "
            f"--rollback-to-fixture     (rolls VM {vm_id()} to {fixture.snapshot!r})"
        )
        lines.append(
            "               NOTE: a rollback DISCARDS the VM's current state. "
            "If what is on it now is worth keeping, snapshot it first."
        )
    if fixture.build:
        lines.append(f"  or build it: {fixture.build}")
    return "\n".join(lines)


# --------------------------------------------------------------------------
# Restoring one (explicit, never a side effect)
# --------------------------------------------------------------------------
def pve_host() -> str:
    return os.environ.get("OPV_PVE_HOST", "root@192.168.0.2")


def vm_id() -> str:
    return os.environ.get("OPV_VM_ID", "104")


def guest() -> str:
    user = os.environ.get("OPV_VM_USER", "PC")
    ip = os.environ.get("OPV_VM_IP", "192.168.0.252")
    return f"{user}@{ip}"


def _ssh(host: str, command: str, timeout: float) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["ssh", "-o", "ConnectTimeout=8", "-o", "StrictHostKeyChecking=accept-new",
         host, command],
        capture_output=True,
        timeout=timeout,
    )


def agent_ready(timeout: float = 120.0, poll_s: float = 3.0) -> bool:
    """Poll the guest's session-1 actuator until it answers, the same way the
    verify script does. A vmstate rollback resumes a live desktop, so this is
    seconds — but it is not instant, and acting into a session that has not
    come back yet looks exactly like a broken app."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            out = _ssh(
                guest(),
                "curl.exe -s -m 2 http://127.0.0.1:8900/health",
                timeout=20.0,
            )
            if b'"ok": true' in out.stdout or b'"ok":true' in out.stdout:
                return True
        except subprocess.TimeoutExpired:
            pass
        time.sleep(poll_s)
    return False


def rollback(name: str, *, wait: bool = True) -> str:
    """Roll the VM back to `name`'s snapshot. Destructive to the live state.

    Callers must have obtained explicit intent first — this function is the
    hand on the lever, not the decision to pull it.
    """
    fixture = FIXTURES.get(name)
    if fixture is None:
        raise RuntimeError(f"unknown fixture {name!r}")
    if not fixture.snapshot:
        raise RuntimeError(
            f"fixture {name!r} has no snapshot to roll back to — it is built by "
            f"hand: {fixture.build}"
        )
    proc = _ssh(pve_host(), f"qm rollback {vm_id()} {fixture.snapshot}", timeout=300.0)
    if proc.returncode != 0:
        raise RuntimeError(
            f"rollback failed ({proc.returncode}): "
            f"{proc.stderr.decode('utf-8', 'replace')[:300]}"
        )
    # `qm start` is the safety net for a snapshot taken WITHOUT vmstate, which
    # comes back stopped. Every fixture here carries vmstate, so the rollback
    # resumes a running VM and the start fails with "already running" — which
    # is the expected outcome, not an error. Chaining the two with `&&` (as the
    # runbook's hand-run recipe does) reports a successful rollback as a
    # failure, which is how this was found: the VM had been put back and the
    # caller was told it had not.
    started = _ssh(pve_host(), f"qm start {vm_id()}", timeout=120.0)
    if started.returncode != 0:
        err = started.stderr.decode("utf-8", "replace")
        if "already running" not in err:
            raise RuntimeError(f"rolled back, but the VM would not start: {err[:300]}")
    if wait and not agent_ready():
        raise RuntimeError(
            f"rolled back to {fixture.snapshot!r} but the session-1 actuator never "
            "answered — the guest may still be resuming, or the agent task did not "
            "start (see deploy_agent.py --status)"
        )
    return fixture.snapshot
