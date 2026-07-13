"""Runner — deploy a daemon, drive the operation catalog to success, run the
global oracles, and produce a structured report.

The report is deployment-agnostic and feeds both the human summary and the
persistent ledger (``opverify.ledger``, phase 1). Each operation is driven
in isolation with its own teardown; cross-cutting health/log/resource
oracles run between operations and at the end.
"""

from __future__ import annotations

import secrets
import time
import traceback
from dataclasses import asdict, dataclass, field
from typing import Any, List, Optional

from . import coverage as cov
from . import oracle as orc
from .catalog import Operation, RunContext
from .client import ClotoClient


@dataclass
class OpResult:
    domain: str
    name: str
    key: str
    passed: bool
    duration_s: float
    covers: List[str] = field(default_factory=list)
    error: Optional[str] = None
    pre_children: Optional[int] = None
    post_children: Optional[int] = None


def _run_op(op: Operation, ctx: RunContext) -> OpResult:
    pre = orc.sample_resources(ctx.target.pid) if ctx.target.pid else orc.ResourceSample()
    t0 = time.monotonic()
    passed, err = True, None
    try:
        result = op.drive(ctx)
        op.assert_success(ctx, result)
    except Exception:  # noqa: BLE001 - any failure is an op failure
        passed = False
        err = traceback.format_exc(limit=4).strip()
    duration = round(time.monotonic() - t0, 3)
    try:
        op.teardown(ctx)
    except Exception:  # noqa: BLE001 - teardown is best-effort
        pass
    post = orc.sample_resources(ctx.target.pid) if ctx.target.pid else orc.ResourceSample()
    return OpResult(
        domain=op.domain,
        name=op.name,
        key=op.key,
        passed=passed,
        duration_s=duration,
        covers=list(op.covers),
        error=err,
        pre_children=pre.child_count,
        post_children=post.child_count,
    )


def run(deployment, ops: List[Operation], ratchet: str = "report") -> dict:
    """Execute a full verification run. ``ratchet`` is ``report`` (list gaps)
    or ``enforce`` (uncovered routes make the run fail)."""
    run_id = f"opverify-{int(time.time())}-{secrets.token_hex(3)}"
    report = orc.OracleReport()
    results: List[OpResult] = []

    target = deployment.start()
    try:
        deployment.wait_ready()
        client = ClotoClient(target.base_url, target.api_key)
        ctx = RunContext(client=client, target=target)

        report.baseline = orc.sample_resources(target.pid) if target.pid else None
        orc.scrape_log(target.stderr_path, report)  # capture boot log baseline

        for op in ops:
            results.append(_run_op(op, ctx))
            orc.check_liveness(client, report)
            orc.scrape_log(target.stderr_path, report)

        # end-of-run cross-cutting checks (while daemon still up)
        orc.check_integrity(client, report)
        report.final = orc.sample_resources(target.pid) if target.pid else None
        orc.scrape_log(target.stderr_path, report)
    finally:
        # final run-time log scrape BEFORE shutdown — teardown naturally
        # closes MCP connections ("MCP Connection closed" / pending-request
        # failures), which are benign shutdown artifacts, not operation
        # failures. Any panic during the actual run is already captured by the
        # per-op and end-of-run scrapes above.
        orc.scrape_log(target.stderr_path, report)
        deployment.stop()

    # post-teardown checks (DB no longer WAL-locked; process gone).
    # Intentionally NOT scraping the log here — see note above.
    orc.check_corruption(target.db_path, report)
    # isolation: prove no real user DB was mutated (seed-mode runs only; a
    # fresh-DB run has no snapshot and leaves isolation_ok = None).
    orc.check_isolation(getattr(deployment, "iso_snapshot", None), report)
    _resource_leak_check(report)

    # release throwaway run state now that all oracles have read from it
    cleanup = getattr(deployment, "cleanup", None)
    if callable(cleanup):
        cleanup()

    # coverage
    kernel_routes = cov.parse_kernel_routes(cov.repo_lib_rs())
    all_covers = [c for op in ops for c in op.covers]
    coverage = cov.compute_coverage(kernel_routes, all_covers)

    passed_ops = sum(1 for r in results if r.passed)
    ops_ok = passed_ops == len(results)
    ratchet_ok = coverage.complete if ratchet == "enforce" else True
    verdict = "pass" if (ops_ok and report.ok and ratchet_ok) else "fail"

    return {
        "run_id": run_id,
        "verdict": verdict,
        "target": {"kind": target.kind, "os": target.os_label},
        "summary": {
            "operations": len(results),
            "passed": passed_ops,
            "failed": len(results) - passed_ops,
        },
        "operations": [asdict(r) for r in results],
        "oracles": report.as_dict(),
        "coverage": coverage.as_dict(),
        "ratchet_mode": ratchet,
    }


def _resource_leak_check(report: orc.OracleReport) -> None:
    if not (report.baseline and report.final):
        return
    b, f = report.baseline.child_count, report.final.child_count
    if b is not None and f is not None and f > b:
        report.note(
            f"resource: child-process count grew {b} -> {f} (possible MCP orphan)"
        )


def print_summary(rep: dict) -> None:
    v = rep["verdict"].upper()
    s = rep["summary"]
    print(f"\n=== opverify {rep['run_id']} — {v} ===")
    print(
        f"target: {rep['target']['kind']} ({rep['target']['os']})  "
        f"ops: {s['passed']}/{s['operations']} passed"
    )
    for op in rep["operations"]:
        mark = "ok " if op["passed"] else "FAIL"
        print(f"  [{mark}] {op['key']:<24} {op['duration_s']:>6.2f}s")
        if not op["passed"] and op["error"]:
            first = op["error"].splitlines()[-1] if op["error"] else ""
            print(f"         -> {first}")
    o = rep["oracles"]
    print(
        f"oracles: liveness={o['liveness_ok']} integrity={o['integrity_ok']} "
        f"log_clean={o['log_clean']} corruption_ok={o['corruption_ok']} "
        f"isolation_ok={o.get('isolation_ok')}"
    )
    for f in o["findings"]:
        print(f"  ! {f}")
    c = rep["coverage"]
    print(
        f"coverage: {c['covered']}/{c['total_routes']} routes ({c['coverage_pct']}%) "
        f"[ratchet={rep['ratchet_mode']}]"
    )
    if c["uncovered"]:
        print(f"  uncovered ({len(c['uncovered'])}): " + ", ".join(c["uncovered"][:12]))
        if len(c["uncovered"]) > 12:
            print(f"    ... and {len(c['uncovered']) - 12} more")
    if c["unknown"]:
        print(f"  unknown (covers not in kernel): " + ", ".join(c["unknown"]))
