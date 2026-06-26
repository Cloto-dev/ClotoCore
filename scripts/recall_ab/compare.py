#!/usr/bin/env python3
"""Compare two recall-A/B result files (e.g. OLD vs NEW arm).

Usage: python compare.py results/results_old.json results/results_new.json
Prints a per-query side-by-side and the aggregate severe-drift delta.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

_SHORT = {"coherent": "C", "mild": "m", "severe": "S", "timeout": "T", "error": "E"}


def _marks(verdicts: list[str]) -> str:
    return "".join(_SHORT.get(v, "?") for v in verdicts)


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    a = json.loads(Path(argv[1]).read_text(encoding="utf-8"))
    b = json.loads(Path(argv[2]).read_text(encoding="utf-8"))
    print(f"A = arm '{a['arm']}'   B = arm '{b['arm']}'   (C=coherent m=mild S=severe T=timeout E=error)\n")
    print(f"{'id':>3}  {'A':<8} {'B':<8}  query")
    for qid, info in a["per_query"].items():
        bm = b["per_query"].get(qid, {}).get("verdicts", [])
        print(f"{qid:>3}  {_marks(info['verdicts']):<8} {_marks(bm):<8}  {info['text']}")
    sa = a["severe_pct_of_completed"]
    sb = b["severe_pct_of_completed"]
    print("\n--- aggregate ---")
    print(f"A severe%: {sa}   counts={a['counts']}")
    print(f"B severe%: {sb}   counts={b['counts']}")
    print(f"delta (B - A): {round(sb - sa, 1)} pp  "
          f"({'improvement' if sb < sa else 'regression' if sb > sa else 'no change'})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
