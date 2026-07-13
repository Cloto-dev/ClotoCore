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

from . import harness
from .catalog import load_all, select


def _build_deployment(args):
    if args.target == "local":
        from .deploy.local import LocalDeployment

        return LocalDeployment(binary=args.binary)
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
    args = p.parse_args(argv)

    operations = select(load_all(), phase0_only=(args.slice == "phase0"))
    if not operations:
        print("no operations selected", file=sys.stderr)
        return 1

    deployment = _build_deployment(args)
    rep = harness.run(deployment, operations, ratchet=args.ratchet)
    harness.print_summary(rep)

    if args.report:
        with open(args.report, "w", encoding="utf-8") as f:
            json.dump(rep, f, indent=2)
        print(f"\nreport written to {args.report}")

    return 0 if rep["verdict"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
