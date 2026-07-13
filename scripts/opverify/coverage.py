"""Coverage ratchet — make "comprehensive" measurable and self-maintaining.

Parse the kernel's route table out of ``crates/core/src/lib.rs`` and cross
it against the routes each catalog operation declares it ``covers``. Any
meaningful route not claimed by some operation is *uncovered*; once the
catalog is complete (phase 1+) the ratchet runs in ``enforce`` mode and a
new uncovered route fails the check — so the catalog cannot silently fall
behind the API. Path parameters are canonicalized (``{id}`` / ``{name}`` →
``{}``) so coverage matching does not depend on parameter spelling.

This mirrors the repo's existing test-count ratchet philosophy.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, List, Set, Tuple

Route = Tuple[str, str]  # (METHOD, canonical_path)

_ROUTE_RE = re.compile(
    r'\.route\(\s*"([^"]+)"\s*,(.*?)(?=\.route\(|\.merge\(|\.layer\(|\.fallback\(|;)',
    re.S,
)
_METHOD_RE = re.compile(r"\b(get|post|put|delete|any)\s*\(")
_PARAM_RE = re.compile(r"\{[^}]*\}")

# Routes that are intentionally NOT operation-catalog targets: static/asset
# serving, raw SSE streams, binary file GETs, and the dynamic plugin proxy.
# (Kept small and explicit — everything else must be covered.)
DEFAULT_IGNORE: Set[Route] = {
    ("GET", "/api/plugin/{}"),  # dynamic proxy passthrough
    ("ANY", "/api/plugin/{}"),
    ("GET", "/api/events"),  # raw SSE stream (publish+history is covered instead)
    ("GET", "/api/marketplace/progress"),  # SSE progress stream
    ("GET", "/api/setup/progress"),  # SSE progress stream
    ("GET", "/api/agents/{}/avatar"),  # binary file serving
    ("POST", "/api/agents/{}/avatar"),
    ("DELETE", "/api/agents/{}/avatar"),
    ("GET", "/api/agents/{}/vrm"),
    ("POST", "/api/agents/{}/vrm"),
    ("DELETE", "/api/agents/{}/vrm"),
    ("GET", "/api/speech/{}"),  # binary file serving
    ("GET", "/api/chat/attachments/{}"),  # binary attachment serving
    ("POST", "/api/system/shutdown"),  # teardown mechanism, not a catalog op
}


def _canon(path: str) -> str:
    return _PARAM_RE.sub("{}", path)


def _full(path: str) -> str:
    return path if path.startswith("/api") else "/api" + path


def parse_kernel_routes(lib_rs: Path) -> Set[Route]:
    text = lib_rs.read_text(encoding="utf-8")
    # bound to the web-server route section to avoid unrelated routers
    start = text.find("let admin_routes = Router::new()")
    end = text.find(".fallback(", start) if start != -1 else -1
    region = text[start:end] if (start != -1 and end != -1) else text
    routes: Set[Route] = set()
    for path, chunk in _ROUTE_RE.findall(region):
        methods = {m.upper() for m in _METHOD_RE.findall(chunk)}
        full = _canon(_full(path))
        for method in methods:
            routes.add((method, full))
    return routes


def _normalize_cover(cover: str) -> Route:
    method, _, path = cover.strip().partition(" ")
    return method.upper(), _canon(path.strip())


@dataclass
class CoverageResult:
    total: int
    covered: int
    uncovered: List[Route] = field(default_factory=list)
    unknown: List[Route] = field(default_factory=list)  # covered but not in kernel
    ignored: int = 0

    @property
    def pct(self) -> float:
        return round(100.0 * self.covered / self.total, 1) if self.total else 100.0

    @property
    def complete(self) -> bool:
        return not self.uncovered and not self.unknown

    def as_dict(self) -> dict:
        return {
            "total_routes": self.total,
            "covered": self.covered,
            "coverage_pct": self.pct,
            "ignored": self.ignored,
            "uncovered": [f"{m} {p}" for m, p in sorted(self.uncovered)],
            "unknown": [f"{m} {p}" for m, p in sorted(self.unknown)],
        }


def compute_coverage(
    kernel_routes: Set[Route],
    covers: Iterable[str],
    ignore: Set[Route] = DEFAULT_IGNORE,
) -> CoverageResult:
    covered_set = {_normalize_cover(c) for c in covers}
    targets = kernel_routes - ignore
    covered = targets & covered_set
    uncovered = targets - covered_set
    unknown = covered_set - kernel_routes - ignore
    return CoverageResult(
        total=len(targets),
        covered=len(covered),
        uncovered=sorted(uncovered),
        unknown=sorted(unknown),
        ignored=len(kernel_routes & ignore),
    )


def repo_lib_rs() -> Path:
    return Path(__file__).resolve().parents[2] / "crates" / "core" / "src" / "lib.rs"
