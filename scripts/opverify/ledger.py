"""Result ledger — the persistent, append-only record of every verification
run, plus regression detection against the prior baseline.

This is permanence mechanism ③ (CSC Goal #169): each run distils the full
report into one compact JSON line appended to ``qa/opverify/history.jsonl``
(committed to the repo, so quality trend is tracked over time). Before
appending, the new entry is compared against the most recent prior entry for
the *same target+os* to surface regressions — an operation that flipped
passing→failing, or a drop in route coverage — which the caller turns into a
red nightly / a report to the maintainer.

Stdlib only; degrades gracefully where ``git`` is unavailable (git_sha=None).
"""

from __future__ import annotations

import json
import os
import secrets
import subprocess
from dataclasses import asdict, dataclass, field
from typing import List, Optional

# repo-root-relative default; resolved against the repo root at call time.
DEFAULT_HISTORY_REL = "qa/opverify/history.jsonl"

# Ledger label for visual-apex runs (scripts/opverify/visual). Deliberately
# disjoint from every harness target kind ("local" / "linux-vm" / "windows-vm"),
# because baselines are matched on (target_kind, os): an apex row is therefore
# only ever compared against a prior apex row. The apex measures a different
# thing (a GUI journey, dual-oracle judged) than the harness (admin-API
# operations + route coverage), so mixing their baselines would produce
# nonsense regressions in both directions.
APEX_TARGET_KIND = "apex"

# The coverage ratchet is a harness concept (kernel routes vs catalog
# operations). The apex has no route catalog, so it declares "n/a" rather than
# borrowing "report"/"enforce" and pretending a ratchet ran.
APEX_RATCHET_MODE = "n/a"


@dataclass
class LedgerEntry:
    """One compact row per run — the shape persisted to history.jsonl."""

    run_id: str
    ts: float  # unix seconds; stamped by the caller (scripts run.py)
    target_kind: str
    os: str
    git_sha: Optional[str]
    verdict: str
    ops_total: int
    ops_passed: int
    failed_ops: List[str] = field(default_factory=list)
    per_domain_pass: dict = field(default_factory=dict)
    coverage_pct: float = 0.0
    covered: int = 0
    total_routes: int = 0
    ratchet_mode: str = "report"
    # Apex-only provenance. Both are part of the baseline key (see
    # _latest_baseline) because rows that differ in either are not comparable:
    #
    #   journey  — different journeys have disjoint step names, so a
    #              "passed before, failing now" diff across two of them is
    #              noise, not a regression.
    #   assessor — "recorded" replays canned visual verdicts, so such a row
    #              carries no visual evidence at all; scoring it against a
    #              "handshake" row (live assessor) compares different things.
    #              Recorded on the row so the committed trend cannot present
    #              the two as equivalent.
    #
    # None on harness rows (and on apex rows written before this field
    # existed, whose assessor is genuinely unknown).
    journey: Optional[str] = None
    assessor: Optional[str] = None


@dataclass
class Regression:
    """A detected quality regression vs the prior same-target baseline."""

    kind: str  # "op-regression" | "coverage-drop"
    detail: str
    baseline_run_id: str


def _repo_root() -> str:
    # scripts/opverify/ledger.py -> parents[2] == repo root
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def git_sha(cwd: Optional[str] = None) -> Optional[str]:
    """Best-effort short HEAD sha; None if git is absent or this is not a repo."""
    try:
        out = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=cwd or _repo_root(),
            capture_output=True,
            text=True,
            timeout=5,
        )
        sha = out.stdout.strip()
        return sha or None
    except (OSError, subprocess.SubprocessError):
        return None


def entry_from_report(
    report: dict, ts: float, sha: Optional[str] = None
) -> LedgerEntry:
    """Distil a full harness report dict into a compact ledger entry."""
    ops = report.get("operations", [])
    per_domain: dict = {}
    for op in ops:
        dom = op.get("domain", "?")
        per_domain[dom] = per_domain.get(dom, True) and bool(op.get("passed"))
    cov = report.get("coverage", {})
    summ = report.get("summary", {})
    tgt = report.get("target", {})
    return LedgerEntry(
        run_id=report.get("run_id", "?"),
        ts=ts,
        target_kind=tgt.get("kind", "?"),
        os=tgt.get("os", "?"),
        git_sha=sha if sha is not None else git_sha(),
        verdict=report.get("verdict", "?"),
        ops_total=summ.get("operations", len(ops)),
        ops_passed=summ.get("passed", sum(1 for o in ops if o.get("passed"))),
        failed_ops=[o["key"] for o in ops if not o.get("passed")],
        per_domain_pass=per_domain,
        coverage_pct=cov.get("coverage_pct", 0.0),
        covered=cov.get("covered", 0),
        total_routes=cov.get("total_routes", 0),
        ratchet_mode=report.get("ratchet_mode", "report"),
    )


def _apex_step_failed(step: dict) -> bool:
    """Did this apex step fail?

    Mirrors ``visual.driver.VisualDriver._tier``: a hard_fail (kernel oracle
    said no) fails, a soft_fail (kernel ok but the screen diverged) fails, and
    an ``error`` step fails too. The error case must be checked explicitly —
    an errored step carries no verdict, so its ``hard_fail`` is ``None``, and
    counting it as a pass would let an actuation/probe crash look clean.
    """
    return bool(step.get("hard_fail") or step.get("soft_fail") or step.get("error"))


def _apex_step_bucket(step: dict) -> str:
    """Grouping key for ``per_domain_pass`` on an apex row.

    The apex has no domains; its meaningful axis is the dual-oracle diagnosis
    (agree_pass / agree_fail / frontend_bug / backend_or_hidden), which is what
    localises a defect to a layer. Steps that never produced a verdict are
    bucketed as ``error``.
    """
    diag = step.get("diagnosis")
    if diag:
        return str(diag)
    return "error" if step.get("error") else "unknown"


def entry_from_apex_report(
    report: dict,
    ts: float,
    os_label: str,
    sha: Optional[str] = None,
    assessor: Optional[str] = None,
) -> LedgerEntry:
    """Distil a visual-apex ``RunReport.as_dict()`` into a compact ledger entry.

    The apex report shape (``visual.driver.RunReport``) differs from the
    harness one, so this is a sibling of :func:`entry_from_report` rather than a
    branch inside it. Steps map onto the ledger's operation fields, so the
    existing op-regression check ("a step that passed last time is failing
    now") applies unchanged. Coverage fields stay 0 — the apex measures no
    route coverage, and 0.0 -> 0.0 can never trip the coverage-drop check.
    """
    steps = report.get("steps", [])
    journey = report.get("journey", "?")
    per_bucket: dict = {}
    for st in steps:
        bucket = _apex_step_bucket(st)
        per_bucket[bucket] = per_bucket.get(bucket, True) and not _apex_step_failed(st)
    failed = [st.get("name", "?") for st in steps if _apex_step_failed(st)]
    return LedgerEntry(
        # The apex driver mints no run id, so one is derived here: the journey
        # names *what* ran, the timestamp orders it, and the random suffix
        # keeps it unique — same shape as the harness id (harness.py mints
        # ``opverify-<epoch>-<hex>``), and for the same reason: whole-second
        # resolution alone collides when two runs land in the same second.
        run_id=f"apex-{journey}-{int(ts)}-{secrets.token_hex(3)}",
        ts=ts,
        target_kind=APEX_TARGET_KIND,
        os=os_label,
        git_sha=sha if sha is not None else git_sha(),
        verdict=report.get("verdict", "?"),
        ops_total=len(steps),
        ops_passed=sum(1 for st in steps if not _apex_step_failed(st)),
        failed_ops=failed,
        per_domain_pass=per_bucket,
        coverage_pct=0.0,
        covered=0,
        total_routes=0,
        ratchet_mode=APEX_RATCHET_MODE,
        journey=journey,
        assessor=assessor,
    )


def load_history(history_path: str) -> List[dict]:
    """Read all prior entries (skips malformed lines rather than failing)."""
    if not os.path.exists(history_path):
        return []
    entries: List[dict] = []
    with open(history_path, "r", encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                entries.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return entries


def _latest_baseline(
    history: List[dict],
    target_kind: str,
    os_label: str,
    journey: Optional[str] = None,
    assessor: Optional[str] = None,
) -> Optional[dict]:
    """Most recent prior entry for the same target+os+journey+assessor (by list
    order, which is append order = chronological).

    ``journey`` and ``assessor`` are None on harness rows, where they match the
    None stored on every other harness row and so change nothing. On apex rows
    they narrow the line to rows that are actually comparable — see the field
    docs on :class:`LedgerEntry`. Legacy apex rows predate both fields, so they
    read back as None and form their own line rather than silently becoming the
    baseline for a run whose provenance is known; the first row of each new
    line establishes it, as with any first run.
    """
    match = [
        e
        for e in history
        if e.get("target_kind") == target_kind
        and e.get("os") == os_label
        and e.get("journey") == journey
        and e.get("assessor") == assessor
    ]
    return match[-1] if match else None


def detect_regressions(entry: LedgerEntry, history: List[dict]) -> List[Regression]:
    """Compare a fresh entry against the latest same-target baseline. A missing
    baseline yields no regressions (first run establishes the line)."""
    base = _latest_baseline(
        history, entry.target_kind, entry.os, entry.journey, entry.assessor
    )
    if base is None:
        return []
    regs: List[Regression] = []
    base_id = base.get("run_id", "?")

    # (a) op passing in baseline but failing now
    base_failed = set(base.get("failed_ops", []))
    now_failed = set(entry.failed_ops)
    newly_failed = sorted(now_failed - base_failed)
    if newly_failed:
        regs.append(
            Regression(
                kind="op-regression",
                detail="ops flipped passing->failing: " + ", ".join(newly_failed),
                baseline_run_id=base_id,
            )
        )

    # (b) coverage dropped (a route lost its owning operation)
    base_cov = float(base.get("coverage_pct", 0.0))
    if entry.coverage_pct + 1e-9 < base_cov:
        regs.append(
            Regression(
                kind="coverage-drop",
                detail=f"route coverage {base_cov}% -> {entry.coverage_pct}%",
                baseline_run_id=base_id,
            )
        )
    return regs


def _append_entry(path: str, entry: LedgerEntry) -> None:
    """Append one row. Append-only by construction: the file is opened in "a"
    mode and never rewritten, so prior rows cannot be lost."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(asdict(entry), ensure_ascii=False) + "\n")


def record(
    report: dict,
    ts: float,
    history_path: Optional[str] = None,
    sha: Optional[str] = None,
) -> tuple[LedgerEntry, List[Regression]]:
    """Distil, detect regressions against the prior baseline, then append.

    Regressions are computed *before* the append so the fresh row is never its
    own baseline. Returns ``(entry, regressions)`` for the caller to report.
    """
    path = history_path or os.path.join(_repo_root(), DEFAULT_HISTORY_REL)
    entry = entry_from_report(report, ts=ts, sha=sha)
    history = load_history(path)
    regressions = detect_regressions(entry, history)

    _append_entry(path, entry)
    return entry, regressions


def record_apex(
    report: dict,
    ts: float,
    os_label: str,
    history_path: Optional[str] = None,
    sha: Optional[str] = None,
    assessor: Optional[str] = None,
) -> tuple[LedgerEntry, List[Regression]]:
    """:func:`record` for a visual-apex run — same ledger, same file, same
    detect-then-append ordering; only the distillation differs.

    ``os_label`` names the OS *under verification* (the VM the GUI runs on),
    not the host that orchestrated the run. ``assessor`` names which visual
    oracle produced the verdicts; pass it so the row cannot be mistaken for
    one gathered by a different oracle.
    """
    path = history_path or os.path.join(_repo_root(), DEFAULT_HISTORY_REL)
    entry = entry_from_apex_report(
        report, ts=ts, os_label=os_label, sha=sha, assessor=assessor
    )
    history = load_history(path)
    regressions = detect_regressions(entry, history)

    _append_entry(path, entry)
    return entry, regressions
