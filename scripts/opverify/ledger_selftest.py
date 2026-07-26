"""Self-test of the result ledger. Run:
``python3 -m scripts.opverify.ledger_selftest`` (exit 0 = passed).

Covers the parts that are only exercised in production by a nightly / an apex
run on a VM, i.e. the parts that would otherwise rot unnoticed:

* apex report -> ledger row distillation (which steps count as passed);
* baseline isolation by ``target_kind`` — an apex row is compared against the
  prior *apex* row, never against a harness row that happens to be adjacent;
* the corollary: apex rows never trip the coverage-drop check (0.0 -> 0.0);
* op-regression detection on apex rows;
* ``record`` / ``record_apex`` really append (prior rows survive).

Stdlib only. Every scenario writes to a fresh temporary directory — the
committed ``qa/opverify/history.jsonl`` is never touched.
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
from dataclasses import asdict

from . import ledger as L

FAKE_SHA = "deadbee"  # explicit, so no scenario shells out to git


# -- fixtures ---------------------------------------------------------------
# Deliberately asymmetric: five steps, three distinct failure modes, and
# pass/fail counts that are neither equal nor a clean half, so an off-by-one or
# an inverted predicate cannot survive by coincidence.
def apex_report(journey: str = "onboarding-advance", verdict: str = "fail") -> dict:
    return {
        "journey": journey,
        "verdict": verdict,
        "steps": [
            {  # clean
                "name": "welcome-rendered",
                "trigger": "checkpoint",
                "diagnosis": "agree_pass",
                "kernel_ok": True,
                "visual_ok": True,
                "hard_fail": False,
                "soft_fail": False,
                "appeared": None,
                "visual_detail": "welcome screen + Get Started button",
                "defects": [],
                "forensic": False,
                "error": None,
            },
            {  # soft: kernel did it, the screen never showed it
                "name": "advance-to-language",
                "trigger": "checkpoint",
                "diagnosis": "frontend_bug",
                "kernel_ok": True,
                "visual_ok": False,
                "hard_fail": False,
                "soft_fail": True,
                "appeared": None,
                "visual_detail": "still on page 1",
                "defects": ["page did not advance"],
                "forensic": True,
                "error": None,
            },
            {  # hard: the kernel oracle said no
                "name": "agent-seeded",
                "trigger": "checkpoint",
                "diagnosis": "agree_fail",
                "kernel_ok": False,
                "visual_ok": False,
                "hard_fail": True,
                "soft_fail": False,
                "appeared": None,
                "visual_detail": None,
                "defects": [],
                "forensic": True,
                "error": None,
            },
            {  # clean
                "name": "settings-opened",
                "trigger": "checkpoint",
                "diagnosis": "agree_pass",
                "kernel_ok": True,
                "visual_ok": True,
                "hard_fail": False,
                "soft_fail": False,
                "appeared": True,
                "visual_detail": "settings panel visible",
                "defects": [],
                "forensic": False,
                "error": None,
            },
            {  # errored: no verdict at all (hard_fail is None, not False)
                "name": "danger-zone-probe",
                "trigger": "checkpoint",
                "diagnosis": None,
                "kernel_ok": None,
                "visual_ok": None,
                "hard_fail": None,
                "soft_fail": None,
                "appeared": None,
                "visual_detail": None,
                "defects": None,
                "forensic": True,
                "error": "kernel probe: ConnectionRefusedError()",
            },
        ],
    }


def clean_apex_report(journey: str = "vm-liveness") -> dict:
    """A two-step all-clean apex run (nothing in ``failed_ops``)."""
    return {
        "journey": journey,
        "verdict": "pass",
        "steps": [
            {
                "name": "app-rendered-and-kernel-healthy",
                "trigger": "checkpoint",
                "diagnosis": "agree_pass",
                "hard_fail": False,
                "soft_fail": False,
                "error": None,
            },
            {
                "name": "settings-opened",
                "trigger": "checkpoint",
                "diagnosis": "agree_pass",
                "hard_fail": False,
                "soft_fail": False,
                "error": None,
            },
        ],
    }


def harness_report(kind: str = "local", os_label: str = "macos") -> dict:
    """A harness report with high coverage and a failing op — the neighbouring
    row an apex baseline must NOT be taken from.

    ``kind``/``os_label`` are parameters because the interesting collision is a
    *kernel* run on the very VM the apex drives (phase 3: ``--target
    windows-vm``): same OS label, different tier. If the fixture only ever put
    a macOS row next to a windows-vm apex row, the OS filter alone would carry
    the test and the target_kind separation would go unverified.
    """
    return {
        "run_id": f"opverify-1700000000-{kind}",
        "verdict": "fail",
        "target": {"kind": kind, "os": os_label},
        "operations": [
            {"key": "health.live", "domain": "health", "passed": True},
            {"key": "agents.lifecycle", "domain": "agents", "passed": False},
            {"key": "memory.read", "domain": "memory", "passed": True},
        ],
        "summary": {"operations": 3, "passed": 2},
        "coverage": {"coverage_pct": 91.5, "covered": 61, "total_routes": 67},
        "ratchet_mode": "report",
    }


def write_history(path: str, entries) -> None:
    with open(path, "w", encoding="utf-8") as fh:
        for e in entries:
            fh.write(json.dumps(e) + "\n")


# -- scenarios --------------------------------------------------------------
def scenario_apex_mapping() -> None:
    e = L.entry_from_apex_report(
        apex_report(), ts=1_700_000_100.0, os_label="windows-vm", sha=FAKE_SHA
    )
    assert e.target_kind == L.APEX_TARGET_KIND, e.target_kind
    assert e.target_kind != "local", "apex rows must not borrow a harness kind"
    assert e.os == "windows-vm", e.os
    assert e.git_sha == FAKE_SHA, e.git_sha
    assert e.verdict == "fail", e.verdict
    assert e.run_id.startswith("apex-onboarding-advance-1700000100-"), e.run_id
    # Unique per run: two rows distilled from the identical report at the
    # identical timestamp must still be distinguishable (a whole-second id
    # collides — the apex was observed minting three identical ids in one
    # process while this was being written).
    twin = L.entry_from_apex_report(
        apex_report(), ts=1_700_000_100.0, os_label="windows-vm", sha=FAKE_SHA
    )
    assert twin.run_id != e.run_id, e.run_id
    assert e.ops_total == 5, e.ops_total
    # clean: welcome-rendered, settings-opened. soft/hard/errored all fail.
    assert e.ops_passed == 2, e.ops_passed
    assert e.failed_ops == [
        "advance-to-language",
        "agent-seeded",
        "danger-zone-probe",
    ], e.failed_ops
    # diagnosis-keyed aggregate: agree_pass clean, the rest not.
    assert e.per_domain_pass == {
        "agree_pass": True,
        "frontend_bug": False,
        "agree_fail": False,
        "error": False,
    }, e.per_domain_pass
    # coverage is meaningless for the apex and must stay zeroed
    assert (e.coverage_pct, e.covered, e.total_routes) == (0.0, 0, 0)
    assert e.ratchet_mode == L.APEX_RATCHET_MODE, e.ratchet_mode


def scenario_errored_step_is_not_a_pass() -> None:
    """An errored step carries hard_fail=None; a truthiness-only check would
    silently count it as clean."""
    rep = clean_apex_report()
    rep["steps"].append(
        {
            "name": "actuator-crashed",
            "trigger": "checkpoint",
            "diagnosis": None,
            "hard_fail": None,
            "soft_fail": None,
            "error": "OSError('ssh: connect failed')",
        }
    )
    e = L.entry_from_apex_report(
        rep, ts=1_700_000_200.0, os_label="windows-vm", sha=FAKE_SHA
    )
    assert e.ops_total == 3, e.ops_total
    assert e.ops_passed == 2, e.ops_passed
    assert e.failed_ops == ["actuator-crashed"], e.failed_ops


def scenario_baseline_isolated_by_target_kind() -> None:
    """Harness rows appended between two apex runs must not become the apex
    baseline — neither for ops nor for coverage. The decisive one is the
    windows-vm kernel row: it shares the apex row's OS label, so only
    ``target_kind`` keeps the two tiers apart."""
    prior_apex = L.entry_from_apex_report(
        clean_apex_report(), ts=1_700_000_000.0, os_label="windows-vm", sha=FAKE_SHA
    )
    local_row = L.entry_from_report(harness_report(), ts=1_700_000_040.0, sha=FAKE_SHA)
    vm_row = L.entry_from_report(
        harness_report(kind="windows-vm", os_label="windows-vm"),
        ts=1_700_000_050.0,
        sha=FAKE_SHA,
    )
    assert vm_row.os == prior_apex.os, "the collision case must share the OS label"
    assert vm_row.target_kind != prior_apex.target_kind
    assert vm_row.coverage_pct == 91.5, vm_row.coverage_pct
    assert vm_row.failed_ops == ["agents.lifecycle"], vm_row.failed_ops

    history = [
        json.loads(json.dumps(asdict(prior_apex))),
        json.loads(json.dumps(asdict(local_row))),
        json.loads(json.dumps(asdict(vm_row))),
    ]

    # (a) an identical apex re-run: no regression, despite the local row's
    #     failing op and its 91.5% coverage sitting right before it.
    same = L.entry_from_apex_report(
        clean_apex_report(), ts=1_700_000_100.0, os_label="windows-vm", sha=FAKE_SHA
    )
    regs = L.detect_regressions(same, history)
    assert regs == [], [r.kind for r in regs]

    # (b) the baseline actually used is the prior apex row.
    worse = L.entry_from_apex_report(
        apex_report(journey="vm-liveness"),
        ts=1_700_000_200.0,
        os_label="windows-vm",
        sha=FAKE_SHA,
    )
    regs = L.detect_regressions(worse, history)
    kinds = [r.kind for r in regs]
    assert kinds == ["op-regression"], kinds
    assert regs[0].baseline_run_id == prior_apex.run_id, regs[0].baseline_run_id
    assert "settings-opened" not in regs[0].detail, regs[0].detail

    # (c) a different OS on the same tier has its own baseline (none yet).
    other_os = L.entry_from_apex_report(
        apex_report(journey="vm-liveness"),
        ts=1_700_000_300.0,
        os_label="linux-vm",
        sha=FAKE_SHA,
    )
    assert L.detect_regressions(other_os, history) == []


def scenario_no_false_coverage_drop() -> None:
    """Apex rows carry coverage 0.0; 0.0 -> 0.0 must never be read as a drop."""
    base = asdict(
        L.entry_from_apex_report(
            clean_apex_report(), ts=1_700_000_000.0, os_label="windows-vm", sha=FAKE_SHA
        )
    )
    now = L.entry_from_apex_report(
        clean_apex_report(), ts=1_700_000_400.0, os_label="windows-vm", sha=FAKE_SHA
    )
    regs = L.detect_regressions(now, [json.loads(json.dumps(base))])
    assert [r.kind for r in regs] == [], [r.kind for r in regs]
    # and the check is live: a real drop on a harness row is still caught
    hr = harness_report()
    hr["coverage"]["coverage_pct"] = 80.0
    dropped = L.entry_from_report(hr, ts=1_700_000_500.0, sha=FAKE_SHA)
    base_local = asdict(L.entry_from_report(harness_report(), ts=1.0, sha=FAKE_SHA))
    kinds = [
        r.kind
        for r in L.detect_regressions(dropped, [json.loads(json.dumps(base_local))])
    ]
    assert kinds == ["coverage-drop"], kinds


def scenario_op_regression_on_apex() -> None:
    """One step that passed last apex run and fails now = exactly one finding,
    naming that step."""
    base = asdict(
        L.entry_from_apex_report(
            clean_apex_report(), ts=1_700_000_000.0, os_label="windows-vm", sha=FAKE_SHA
        )
    )
    rep = clean_apex_report()
    rep["verdict"] = "warn"
    rep["steps"][1].update(
        {"diagnosis": "frontend_bug", "hard_fail": False, "soft_fail": True}
    )
    now = L.entry_from_apex_report(
        rep, ts=1_700_000_600.0, os_label="windows-vm", sha=FAKE_SHA
    )
    regs = L.detect_regressions(now, [json.loads(json.dumps(base))])
    assert len(regs) == 1, [r.kind for r in regs]
    assert regs[0].kind == "op-regression", regs[0].kind
    assert "settings-opened" in regs[0].detail, regs[0].detail
    assert "app-rendered-and-kernel-healthy" not in regs[0].detail, regs[0].detail


def scenario_record_appends() -> None:
    """``record`` / ``record_apex`` append to the same file without disturbing
    prior rows — and regressions are computed before the append, so a fresh row
    is never its own baseline."""
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "nested", "history.jsonl")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        sentinel = {"run_id": "pre-existing", "target_kind": "local", "os": "macos"}
        write_history(path, [sentinel])

        e1, r1 = L.record_apex(
            clean_apex_report(),
            ts=1_700_000_000.0,
            os_label="windows-vm",
            history_path=path,
            sha=FAKE_SHA,
        )
        assert r1 == [], "first apex row establishes the line"
        e2, r2 = L.record(
            harness_report(), ts=1_700_000_050.0, history_path=path, sha=FAKE_SHA
        )
        e3, r3 = L.record_apex(
            apex_report(journey="vm-liveness"),
            ts=1_700_000_100.0,
            os_label="windows-vm",
            history_path=path,
            sha=FAKE_SHA,
        )
        assert [r.kind for r in r3] == ["op-regression"], [r.kind for r in r3]
        assert r3[0].baseline_run_id == e1.run_id, r3[0].baseline_run_id

        rows = L.load_history(path)
        assert len(rows) == 4, len(rows)
        assert rows[0] == sentinel, rows[0]
        assert [r["run_id"] for r in rows[1:]] == [
            e1.run_id,
            e2.run_id,
            e3.run_id,
        ], rows
        assert rows[1]["target_kind"] == L.APEX_TARGET_KIND
        assert rows[2]["target_kind"] == "local"

        # a fourth run re-reads the file it just grew (no truncation)
        L.record_apex(
            clean_apex_report(),
            ts=1_700_000_200.0,
            os_label="windows-vm",
            history_path=path,
            sha=FAKE_SHA,
        )
        assert len(L.load_history(path)) == 5


def scenario_repo_history_untouched() -> None:
    """Guard on the guard: the committed ledger must not grow while this
    selftest runs (a scenario that forgot ``history_path`` would write to it)."""
    real = os.path.join(L._repo_root(), L.DEFAULT_HISTORY_REL)
    before = os.path.getsize(real) if os.path.exists(real) else None
    with tempfile.TemporaryDirectory() as tmp:
        L.record_apex(
            clean_apex_report(),
            ts=1_700_000_900.0,
            os_label="windows-vm",
            history_path=os.path.join(tmp, "history.jsonl"),
            sha=FAKE_SHA,
        )
    after = os.path.getsize(real) if os.path.exists(real) else None
    assert before == after, f"committed ledger changed: {before} -> {after}"


def main() -> int:
    scenarios = [
        scenario_apex_mapping,
        scenario_errored_step_is_not_a_pass,
        scenario_baseline_isolated_by_target_kind,
        scenario_no_false_coverage_drop,
        scenario_op_regression_on_apex,
        scenario_record_appends,
        scenario_repo_history_untouched,
    ]
    for sc in scenarios:
        sc()
        print(f"  ok  {sc.__name__}")
    print(f"ledger selftest: {len(scenarios)}/{len(scenarios)} scenarios passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
