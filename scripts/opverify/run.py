"""opverify CLI entrypoint.

    python -m scripts.opverify.run --target local [--slice phase0]
                                   [--binary PATH] [--ratchet report|enforce]
                                   [--report out.json]

Exit code: 0 if the run's verdict is ``pass``, 1 otherwise — so the same
invocation is usable as a CI gate and as a pre-stable-cut manual check.
"""

from __future__ import annotations

import argparse
import json
import sys
import time

from . import harness
from . import ledger as ledger_mod
from .catalog import load_all, select


def _default_seed_db() -> str | None:
    """The operator's dev DB — carries real LLM keys for chat verification.
    Copied (never read) into a rewired throwaway by bootstrap.py."""
    import os

    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    for rel in (
        "target/debug/data/cloto_memories.db",
        "target/release/data/cloto_memories.db",
    ):
        p = os.path.join(root, rel)
        if os.path.exists(p):
            return p
    return None


def _build_deployment(args, seed_db):
    if args.target == "local":
        from .deploy.local import LocalDeployment

        return LocalDeployment(binary=args.binary, seed_db=seed_db)
    if args.target in ("linux-vm", "windows-vm"):
        raise SystemExit(
            f"target '{args.target}' not yet implemented (phase 2/3)"
        )
    raise SystemExit(f"unknown target: {args.target}")


def main(argv=None) -> int:
    p = argparse.ArgumentParser(prog="opverify", description=__doc__)
    p.add_argument("--target", default="local",
                   choices=["local", "linux-vm", "windows-vm"])
    p.add_argument("--slice", default="all", choices=["all", "phase0"],
                   help="operation subset to run")
    p.add_argument("--binary", default=None,
                   help="path to clotocore binary (local target)")
    p.add_argument("--ratchet", default="report", choices=["report", "enforce"])
    p.add_argument("--report", default=None, help="write JSON report to this path")
    p.add_argument(
        "--seed-db",
        default=None,
        help="DB to copy+rewire for real chat verification (default: auto-detect "
        "the dev DB when chat ops are selected). Keys ride along, never read.",
    )
    p.add_argument(
        "--no-seed",
        action="store_true",
        help="force a fresh empty DB even when chat ops are selected (chat ops "
        "will then fail — use with --slice phase0)",
    )
    p.add_argument(
        "--ledger",
        action="store_true",
        help="append this run to qa/opverify/history.jsonl and check for "
        "regressions vs the prior same-target baseline (pass in nightly/VM "
        "runs; omit for local dev iteration so the committed trend stays clean)",
    )
    p.add_argument(
        "--history",
        default=None,
        help="override ledger history path (default qa/opverify/history.jsonl)",
    )
    args = p.parse_args(argv)

    operations = select(load_all(), phase0_only=(args.slice == "phase0"))
    if not operations:
        print("no operations selected", file=sys.stderr)
        return 1

    # Seed mode: needed only when a chat op (real LLM) is in the selected set.
    has_chat = any(op.domain == "chat" for op in operations)
    seed_db = None
    if not args.no_seed and has_chat:
        seed_db = args.seed_db or _default_seed_db()
        if seed_db:
            print(f"seed: chat ops enabled — copying+rewiring {seed_db}")
        else:
            print(
                "seed: chat ops selected but no dev DB found — chat will fail; "
                "pass --seed-db PATH or --slice phase0",
                file=sys.stderr,
            )

    deployment = _build_deployment(args, seed_db)
    rep = harness.run(deployment, operations, ratchet=args.ratchet)
    harness.print_summary(rep)

    if args.report:
        with open(args.report, "w", encoding="utf-8") as f:
            json.dump(rep, f, indent=2)
        print(f"\nreport written to {args.report}")

    regressed = False
    if args.ledger:
        entry, regressions = ledger_mod.record(
            rep, ts=time.time(), history_path=args.history
        )
        print(
            f"\nledger: recorded {entry.run_id} "
            f"(git={entry.git_sha or 'n/a'}, coverage={entry.coverage_pct}%)"
        )
        for reg in regressions:
            regressed = True
            print(f"  !! REGRESSION [{reg.kind}] {reg.detail} "
                  f"(vs {reg.baseline_run_id})")

    ok = rep["verdict"] == "pass" and not regressed
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
