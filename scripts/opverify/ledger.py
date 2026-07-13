"""Result ledger — the persistent, append-only record of every verification
run, plus regression detection against the prior baseline.

This is permanence mechanism ③ (an earlier decision): each run distils the full
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
import subprocess
from dataclasses import asdict, dataclass, field
from typing import List, Optional

# repo-root-relative default; resolved against the repo root at call time.
DEFAULT_HISTORY_REL = "qa/opverify/history.jsonl"


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
    history: List[dict], target_kind: str, os_label: str
) -> Optional[dict]:
    """Most recent prior entry for the same target+os (by list order, which is
    append order = chronological)."""
    match = [
        e
        for e in history
        if e.get("target_kind") == target_kind and e.get("os") == os_label
    ]
    return match[-1] if match else None


def detect_regressions(entry: LedgerEntry, history: List[dict]) -> List[Regression]:
    """Compare a fresh entry against the latest same-target baseline. A missing
    baseline yields no regressions (first run establishes the line)."""
    base = _latest_baseline(history, entry.target_kind, entry.os)
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

    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(asdict(entry), ensure_ascii=False) + "\n")
    return entry, regressions
