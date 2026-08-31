#!/usr/bin/env python3
"""Verify that measurable facts stated in the docs match the source of truth.

Hand-written numbers rot, and nothing in this repository noticed. Measured when
this gate was written: the README badge claimed 234 passing tests against a
suite of 666, the contributor guide called the current phase `0.6.3-alpha.11`
while the version was `0.6.8-beta.5`, the support policy named a pre-release
four cuts behind, and one documented environment default no longer matched the
code. Every one of those had been green in CI for months, because no check
existed that could go red.

Checked facts and their sources of truth:

  test counts     the test attributes in `crates/` and the test cases in
                  `dashboard/src`, counted statically. Cross-checked against
                  `qa/test-baseline.json`, which the ratchet fills in from
                  actual `cargo test` / `npm test` output: on the day this was
                  written the two independent methods agreed exactly (666 and
                  65), which is what makes the cheap static count usable here
                  instead of running both suites for a documentation check.

  current version `version` in the workspace `Cargo.toml`, compared against
                  every doc that names the phase the project is currently in.

  release names   git tags, compared against "Latest release: vX.Y.Z" claims
                  and against any pre-release version the docs name. Version
                  freshness is the one fact that rots on a schedule — it goes
                  stale the moment a tag is cut, with no edit anywhere to
                  trigger a review.

  env defaults    a static parse of `env::var("NAME").unwrap_or_else(...)`
                  across `crates/`, compared against every markdown table row
                  in the scanned docs that documents a variable. A documented
                  variable that no longer appears in the source at all is also
                  a finding — that is how a removed setting keeps being
                  documented.

Deliberately NOT checked, and why:

  * The tool count of the memory server, named in the README's plugin table.
    Its source of truth is another repository, so any number here is a copy
    that cannot be verified locally. The number was removed rather than gated.
  * The kernel tool count in the architecture document's file tree. Measured
    while writing this: the file defines 15 `mgp.*` tool literals plus one
    declared as a constant, and two `gui.*` tools — so the "18" was the file's
    total, labelled as the `mgp.*` namespace count. A grep-based gate would
    have to encode which literals are registrations, which is exactly the
    shape that miscounts; and a Rust test already asserts the real registered
    total. The number was removed from the annotation instead.

Point-in-time documents are not scanned at all — `*_DESIGN.md`, the changelog,
and the documentation policy state what was true when they were written, and a
statement like "31 files, 578,379 characters" is a record, not a claim about
now. Scanning them would make this gate demand that history be rewritten.

Exit 0 when every claim holds; exit 1 with one line per violation otherwise.
Run from the repository root: `python3 scripts/check-docs-facts.py`.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Docs that claim CURRENT state and therefore must track the code.
DOC_FILES = [
    ROOT / "README.md",
    ROOT / "SUPPORT.md",
    ROOT / "SECURITY.md",
    ROOT / "CONTRIBUTING.md",
    ROOT / "docs" / "index.md",
    ROOT / "docs" / "ARCHITECTURE.md",
    ROOT / "docs" / "DEVELOPMENT.md",
    ROOT / "docs" / "QUICKSTART_MCP_SERVER.md",
    ROOT / "docs" / "PROJECT_VISION.md",
]

# Relative drift allowed on the test counts before the gate goes red. The docs
# may state them as rounded `~` values; 3% keeps a handful of new tests from
# forcing a documentation edit per commit, while the 65% drift this gate was
# built for (234 stated against 666 measured) fails several times over.
TOLERANCE = 0.03

failures: list[str] = []


def fail(msg: str) -> None:
    failures.append(msg)


def rel(path: Path) -> str:
    return str(path.relative_to(ROOT))


# --- sources of truth -------------------------------------------------------

RUST_TEST_ATTR = re.compile(r"^\s*#\[(?:tokio::)?test\]", re.M)
# `it("...")` / `test("...")` at a call position. `describe` is a grouping call
# and is deliberately not counted; vitest reports the leaf cases.
TS_TEST_CASE = re.compile(r"\b(?:it|test)\s*\(\s*[\"'`]")


def measured_test_counts() -> dict[str, int]:
    """{'rust': n, 'dashboard': m}, counted statically from the sources.

    Static counting is a proxy for what the suites actually run, so it is
    cross-checked below against the ratchet's recorded numbers, which come from
    real test-run output. If the two ever disagree the proxy has stopped being
    one, and this gate says so rather than quietly grading the docs against a
    number that no longer means what its name says.
    """
    rust = sum(
        len(RUST_TEST_ATTR.findall(p.read_text(errors="replace")))
        for p in sorted((ROOT / "crates").rglob("*.rs"))
    )
    dash = 0
    for pattern in ("*.test.ts", "*.test.tsx"):
        for p in sorted((ROOT / "dashboard" / "src").rglob(pattern)):
            dash += len(TS_TEST_CASE.findall(p.read_text(errors="replace")))
    return {"rust": rust, "dashboard": dash}


def cross_check_against_ratchet(counts: dict[str, int]) -> None:
    baseline_file = ROOT / "qa" / "test-baseline.json"
    if not baseline_file.is_file():
        fail(f"{rel(baseline_file)}: missing — cannot validate the static test count")
        return
    baseline = json.loads(baseline_file.read_text())
    for key, field in (("rust", "rust_test_count"), ("dashboard", "dashboard_test_count")):
        recorded = baseline.get(field)
        if not isinstance(recorded, int):
            fail(f"{rel(baseline_file)}: {field} is not a number")
            continue
        # The ratchet floor is raised by hand, so it is allowed to lag behind a
        # growing suite. It must never EXCEED it: that would mean the static
        # count is missing tests the runner sees, and the docs would then be
        # graded against an undercount.
        if counts[key] < recorded:
            fail(
                f"{rel(baseline_file)}: {field}={recorded} exceeds the static count "
                f"({counts[key]}) — the static counter is missing tests the runner "
                f"sees, so it cannot be used to check the documented count"
            )


def git(*args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=ROOT, capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        fail(f"git {' '.join(args)} failed: {result.stderr.strip()}")
        return ""
    return result.stdout.strip()


def workspace_version() -> str | None:
    """The `version` under `[workspace.package]` in the root Cargo.toml.

    Scoped to that section rather than taken as the first line-anchored
    `version =` in the file. The unscoped form happens to work today only
    because `[workspace.package]` is written above `[workspace.dependencies]`
    and every dependency uses an inline table (`sqlx = { version = "0.9" }`),
    which is not line-anchored. Both of those are conventions, not guarantees:
    a dependency written in expanded form
    (`[workspace.dependencies.sqlx]` / `version = "0.9"`) above the package
    section makes the unscoped search return the dependency's version, and the
    gate would then grade every document against it — a wrong answer with no
    symptom, since the comparison still runs and can still pass.
    """
    cargo = (ROOT / "Cargo.toml").read_text()
    section = None
    for line in cargo.splitlines():
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            section = stripped[1:-1]
            continue
        if section != "workspace.package":
            continue
        m = re.match(r'version\s*=\s*"([^"]+)"', stripped)
        if m:
            return m.group(1)
    fail("Cargo.toml: no version under [workspace.package] — update this script")
    return None


def measured_versions() -> dict[str, str]:
    """Current version from Cargo.toml, plus the newest final and pre-release tags."""
    current = workspace_version()
    if current is None:
        return {}

    tags = [t for t in git("tag", "--sort=-v:refname").splitlines() if t.startswith("v")]
    if not tags:
        # A shallow or tagless clone must be an error, never a skip: with no
        # tags every release claim would pass by default, and the check would
        # be loudest exactly when it had stopped looking. CI fetches tags.
        fail(
            "no version tags found — fetch tags (actions/checkout needs "
            "fetch-depth: 0 or fetch-tags: true); release claims cannot be checked"
        )
        return {"current": current}

    finals = [t for t in tags if re.fullmatch(r"v\d+\.\d+\.\d+", t)]
    prereleases = [t for t in tags if re.fullmatch(r"v\d+\.\d+\.\d+-[\w.]+", t)]
    out = {"current": current}
    if finals:
        out["latest_final"] = finals[0]
    if prereleases:
        out["latest_prerelease"] = prereleases[0]
    return out


# `env::var("NAME")` … `unwrap_or_else(|_| "DEFAULT".to_string())`. Other
# shapes (`.ok()`, `is_ok()`, parsed enums) carry no string default and are
# covered by the existence check instead of the value check.
ENV_WITH_DEFAULT = re.compile(
    r'env::var\("([A-Z0-9_]+)"\)\s*(?:\.ok\(\))?[^;]*?'
    r'unwrap_or_else\(\|_\|\s*"([^"]*)"\.to_string\(\)\)',
    re.S,
)


# Where a variable name may legitimately appear. Documentation is deliberately
# absent: a doc cannot be its own evidence that the setting exists.
NAME_SOURCES = (
    ("crates", "*.rs"),
    ("dashboard/src-tauri", "*.rs"),
    ("scripts", "*"),
    (".github", "*"),
)


def measured_env() -> tuple[dict[str, str], set[str]]:
    """({VAR: default}, {every VAR the project mentions outside its docs}).

    The default map comes only from the `env::var(...).unwrap_or_else(...)`
    shape in Rust, which is what carries a comparable value.

    Existence is a wider question and needs a wider oracle. Measured while
    writing this: reading `env::var` only would have reported six live
    variables as dead. Provider keys reach the code as entries in a mapping
    table (`("deepseek", "DEEPSEEK_API_KEY")`), `RUST_LOG` is consumed by the
    logging library and appears here only in a doc comment, and several are
    pass-through settings for plugin servers whose registry is `.env.example`
    — the file the README tells users to copy. So a name counts as live if the
    project mentions it anywhere outside the documentation being checked.
    """
    defaults: dict[str, str] = {}
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        for name, default in ENV_WITH_DEFAULT.findall(path.read_text(errors="replace")):
            defaults.setdefault(name, default)

    seen: set[str] = set()
    haystack: list[str] = []
    env_example = ROOT / ".env.example"
    if env_example.is_file():
        haystack.append(env_example.read_text(errors="replace"))
    else:
        fail(".env.example is missing — it is the registry for pass-through settings")
    for subdir, pattern in NAME_SOURCES:
        base = ROOT / subdir
        if not base.is_dir():
            continue
        for path in sorted(base.rglob(pattern)):
            if path.is_file():
                haystack.append(path.read_text(errors="replace"))
    blob = "\n".join(haystack)
    seen.update(re.findall(r"\b([A-Z][A-Z0-9_]{3,})\b", blob))
    return defaults, seen


# --- claims stated in the docs ----------------------------------------------

BADGE_TESTS = re.compile(r"tests-(\d+)(?:%20| )passing")
SPLIT_TESTS = re.compile(r"Rust\s+(\d+)\s*\+\s*Dashboard\s+(\d+)")
TOTAL_TESTS = re.compile(r"~?(\d+)\s+tests\s*\(Rust")
# A count that names the suite it counts: `Rust (234 tests)`, `Dashboard: 65
# tests`. Naming the suite is the line between a claim about now and a record
# of then — the contributor guide's `# Rust (234 tests)` is an instruction and
# was 432 short, while "All 11 tests passing" in a table of completed audit
# items names no suite and is a record of what that audit verified. Asking
# prose which of its numbers are current would repeat the mistake the version
# markers exist to avoid.
SUITE_COUNT = re.compile(
    r"\b(Rust|Dashboard)\b\s*:?\s*\(?~?(\d+)\s+tests?\b", re.I
)
CURRENT_PHASE = re.compile(r"Current\s*\((\d+\.\d+\.\d+(?:-[\w.]+)?)\)")
LATEST_RELEASE = re.compile(r"Latest release:\s*(v?\d+\.\d+\.\d+)")
# A version literal followed by an explicit marker naming which moving target
# it is supposed to track:
#
#     0.6.8 pre-releases (0.6.8-beta.5 <!-- docs-facts: latest-prerelease -->)
#
# A blanket scan for pre-release-shaped strings was tried first and had to be
# withdrawn: it fired on the support policy's line explaining semver notation
# ("`0.6.8-alpha.1`, `0.6.8-beta.1`, `0.6.8-rc.1`"), which teaches the format
# and is not a claim about what is current. Prose cannot be asked which of its
# version numbers are assertions, so the document says so. The cost is that a
# new claim is only covered once someone marks it — which is visible in the
# source, unlike a checker that silently guessed wrong.
MARKED_VERSION = re.compile(
    r"(v?\d+\.\d+\.\d+(?:-[\w.]+)?)\s*<!--\s*docs-facts:\s*([a-z-]+)\s*-->"
)
MARKER_SOURCES = {
    "latest-prerelease": "latest_prerelease",
    "latest-release": "latest_final",
    "current-version": "current",
}
ENV_ROW = re.compile(r"^\|\s*`([A-Z0-9_]+)`\s*\|\s*([^|]*?)\s*\|", re.M)


def within_tolerance(stated: int, measured: int) -> bool:
    if measured == 0:
        return stated == 0
    return abs(stated - measured) / measured <= TOLERANCE


def check_document(path: Path, counts: dict[str, int], versions: dict[str, str],
                   env_defaults: dict[str, str], env_seen: set[str]) -> None:
    text = path.read_text(errors="replace")
    name = rel(path)

    for match in BADGE_TESTS.finditer(text):
        stated = int(match.group(1))
        if not within_tolerance(stated, counts["rust"]):
            fail(f"{name}: test badge says {stated}, measured {counts['rust']} Rust tests")

    for match in SPLIT_TESTS.finditer(text):
        for stated, measured, label in (
            (int(match.group(1)), counts["rust"], "Rust"),
            (int(match.group(2)), counts["dashboard"], "Dashboard"),
        ):
            if not within_tolerance(stated, measured):
                fail(f"{name}: says {stated} {label} tests, measured {measured}")

    for match in SUITE_COUNT.finditer(text):
        suite = match.group(1).lower()
        stated = int(match.group(2))
        measured = counts["rust" if suite == "rust" else "dashboard"]
        if not within_tolerance(stated, measured):
            fail(
                f"{name}: says {stated} {match.group(1)} tests, measured {measured}"
            )

    total = counts["rust"] + counts["dashboard"]
    for match in TOTAL_TESTS.finditer(text):
        stated = int(match.group(1))
        if not within_tolerance(stated, total):
            fail(f"{name}: says {stated} tests in total, measured {total}")

    if "current" in versions:
        for match in CURRENT_PHASE.finditer(text):
            if match.group(1) != versions["current"]:
                fail(
                    f"{name}: names {match.group(1)} as the current version, "
                    f"Cargo.toml says {versions['current']}"
                )

    if "latest_final" in versions:
        for match in LATEST_RELEASE.finditer(text):
            stated = match.group(1).lstrip("v")
            if stated != versions["latest_final"].lstrip("v"):
                fail(
                    f"{name}: says the latest release is {match.group(1)}, "
                    f"the newest final tag is {versions['latest_final']}"
                )

    for match in MARKED_VERSION.finditer(text):
        stated, marker = match.group(1), match.group(2)
        key = MARKER_SOURCES.get(marker)
        if key is None:
            fail(f"{name}: unknown docs-facts marker `{marker}`")
            continue
        if key not in versions:
            continue  # the source of truth was unavailable and already reported
        if stated.lstrip("v") != versions[key].lstrip("v"):
            fail(
                f"{name}: `{stated}` is marked {marker}, "
                f"but that is {versions[key]}"
            )

    for var, stated in ENV_ROW.findall(text):
        if var not in env_seen:
            fail(
                f"{name}: documents {var}, which nothing in the source, "
                f".env.example, scripts or workflows mentions"
            )
            continue
        if var not in env_defaults:
            continue  # no string default in the source; nothing to compare
        value = stated.strip().strip("`")
        if value in ("(none)", "-", ""):
            continue  # documented as unset, which the value check cannot judge
        if value != env_defaults[var]:
            fail(
                f"{name}: documents {var} default as `{value}`, "
                f"the source says `{env_defaults[var]}`"
            )


def main() -> int:
    counts = measured_test_counts()
    cross_check_against_ratchet(counts)
    versions = measured_versions()
    env_defaults, env_seen = measured_env()

    if not env_seen:
        fail("no env::var reads found under crates/ — update this script")

    scanned = 0
    for path in DOC_FILES:
        if not path.is_file():
            fail(f"{rel(path)}: listed as a scanned document but does not exist")
            continue
        scanned += 1
        check_document(path, counts, versions, env_defaults, env_seen)

    if failures:
        for f in failures:
            print(f"::error::docs-facts: {f}")
            print(f"  - {f}", file=sys.stderr)
        print(f"{len(failures)} stale claim(s) in {scanned} document(s)", file=sys.stderr)
        return 1

    print(
        f"docs facts: OK ({scanned} documents checked against "
        f"{counts['rust']}+{counts['dashboard']} tests, version "
        f"{versions.get('current', '?')}, {len(env_defaults)} env defaults)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
