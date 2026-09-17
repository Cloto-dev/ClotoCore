"""Self-test of the route inventory the coverage ratchet counts against. Run:
``python3 -m scripts.opverify.coverage_selftest`` (exit 0 = passed).

The ratchet is only as honest as ``parse_kernel_routes``: a method router the
parser does not know is dropped from the inventory silently — its routes are
neither covered nor uncovered, and an operation that covers one is reported as
covering a route the kernel does not have. PATCH was such a verb until the
conversation routes introduced the first ``patch(...)``.

Two checks. A synthetic router with every verb — its last route directly
before `.fallback(`, where the region is cut — proves the parser reads each
one and keeps the last route. The real ``crates/core/src/lib.rs`` is then read by an independent, wider
pattern — every axum method-router name, including ones the kernel does not use
today — and every verb it finds on a route must be in the parser's inventory,
so the next new verb fails here instead of disappearing.

Stdlib only; reads files, writes nothing.
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

from . import coverage as C

_ROOT = Path(__file__).resolve().parents[2]
_LIB_RS = _ROOT / "crates" / "core" / "src" / "lib.rs"

# Every method router axum exposes (axum::routing::*), wider than the parser's.
_ALL_VERBS = re.compile(r"\b(get|post|put|patch|delete|head|options|trace|connect|any)\s*\(")

_SYNTHETIC = """
fn build() {
    let admin_routes = Router::new()
        .route("/items", get(list).post(create))
        .route("/items/{id}", patch(update).delete(remove).put(replace))
        .route("/anything", any(handler))
        .fallback(nothing);
}
"""


def _fail(msg: str) -> None:
    print(f"FAIL: {msg}")
    sys.exit(1)


def check_synthetic() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        lib = Path(tmp) / "lib.rs"
        lib.write_text(_SYNTHETIC, encoding="utf-8")
        got = C.parse_kernel_routes(lib)
    want = {
        ("GET", "/api/items"),
        ("POST", "/api/items"),
        ("PATCH", "/api/items/{}"),
        ("DELETE", "/api/items/{}"),
        ("PUT", "/api/items/{}"),
        ("ANY", "/api/anything"),
    }
    if got != want:
        _fail(f"synthetic router parsed as {sorted(got)}, expected {sorted(want)}")
    print("ok: every verb of a synthetic router is read")


def check_real_lib() -> None:
    text = _LIB_RS.read_text(encoding="utf-8")
    start = text.find("let admin_routes = Router::new()")
    end = text.find(".fallback(", start)
    if start == -1 or end == -1:
        _fail("could not find the admin router in lib.rs — the parser's anchor moved")
    region = text[start:end]
    inventory = C.parse_kernel_routes(_LIB_RS)
    missing = []
    seen_routes = 0
    for path, chunk in C._ROUTE_RE.findall(region):
        seen_routes += 1
        full = C._canon(C._full(path))
        for verb in {v.upper() for v in _ALL_VERBS.findall(chunk)}:
            if (verb, full) not in inventory:
                missing.append(f"{verb} {full}")
    if seen_routes == 0:
        _fail("read no routes from lib.rs — this check would pass vacuously")
    declared = len(re.findall(r"\.route\(\s*\"", region))
    if declared != seen_routes:
        _fail(f"lib.rs declares {declared} routes in the admin router, the parser read {seen_routes}")
    if missing:
        _fail("routes in lib.rs the inventory does not count: " + ", ".join(sorted(missing)))
    print(f"ok: all verbs on {seen_routes} route declarations in lib.rs are counted ({len(inventory)} routes)")


def main() -> int:
    check_synthetic()
    check_real_lib()
    print("coverage self-test passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
