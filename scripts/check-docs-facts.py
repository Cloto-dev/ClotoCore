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

  route claims    the `**Route:** `METHOD /path`` and `/// METHOD /api/path`
                  lines in `crates/core/src/handlers/`, compared against the
                  routes `lib.rs` actually mounts. A handler's own doc comment
                  is what an operator reads to call it, and a renamed or
                  re-verbed route leaves that comment behind with nothing to
                  notice: the code compiles, the tests pass, and the sentence
                  describing the endpoint is simply wrong. Measured when this
                  was added: six such claims had drifted and been repaired by
                  hand, with nothing to stop the seventh.

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

sys.path.insert(0, str(Path(__file__).resolve().parent))
from version_display import to_display  # noqa: E402

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
# Both spellings must be *recognised* here even though only one is accepted
# below. A claim the pattern does not match is not a claim that fails — it is a
# claim that stops being graded, silently, which is the failure this file
# exists to prevent. The display spelling (`0.6.9a1`) has no hyphen before its
# stage, so the older alternative alone would have skipped straight past every
# line the new convention produces.
MARKED_VERSION = re.compile(
    r"(v?\d+\.\d+\.\d+(?:-[\w.]+|(?:a|b|rc)\d+)?)\s*<!--\s*docs-facts:\s*([a-z-]+)\s*-->"
)
MARKER_SOURCES = {
    "latest-prerelease": "latest_prerelease",
    "latest-release": "latest_final",
    "current-version": "current",
}
ENV_ROW = re.compile(r"^\|\s*`([A-Z0-9_]+)`\s*\|\s*([^|]*?)\s*\|", re.M)


# --- routes: what the handler docs claim vs what lib.rs mounts --------------

# Every route in this kernel is nested under `/api`, so a mounted path is the
# literal in `.route(...)` with that prefix. `.route` calls span lines and chain
# methods (`post(..).delete(..)`), so the method set is read from the balanced
# argument rather than from the same line.
_ROUTE_CALL = re.compile(
    r"""\.route\(\s*(?:"(?P<lit>[^"]+)"|(?P<const>[A-Za-z_][\w:]*))\s*,""", re.S
)
_METHOD_CALL = re.compile(r"\b(get|post|put|delete|patch)\s*\(")
_PATH_CONST = re.compile(r'pub const (\w+): &str = "([^"]+)"')

# Two spellings are in use, and both have to be recognised: a claim the pattern
# misses is not one that fails, it is one that stops being graded.
_ROUTE_CLAIMS = (
    re.compile(r"^\s*///\s*\*\*Route:\*\*\s*`(?P<method>[A-Z]+)\s+(?P<path>/[^`\s]*)`"),
    re.compile(r"^\s*///\s*(?P<method>GET|POST|PUT|DELETE|PATCH)\s+(?P<path>/api/\S*)"),
)


def route_shape(path: str) -> str:
    """Reduce a path to what both sides must agree on.

    Placeholder *names* are documentation: a comment that calls a segment
    `:approval_id` where the route mounts `{id}` describes the same endpoint,
    and grading the spelling reports nine such pairs as drift — enough to bury
    a real finding. A documented query string (`/x[?a=b]`) describes arguments,
    not a different path.
    """
    path = path.split("[", 1)[0].split("?", 1)[0]
    path = re.sub(r"\{\*[^}]*\}|\*[A-Za-z_]\w*", "{*}", path)
    path = re.sub(r"\{[^}*]*\}|:[A-Za-z_]\w*", "{}", path)
    return path.rstrip("/") or "/"


def mounted_routes() -> dict[str, set[str]]:
    """`{shape: {METHOD}}` for every route the kernel mounts."""
    src = ROOT / "crates" / "core" / "src"
    lib = (src / "lib.rs").read_text(encoding="utf-8")
    middleware = src / "middleware.rs"
    consts = {
        f"middleware::{name}": value
        for name, value in _PATH_CONST.findall(
            middleware.read_text(encoding="utf-8") if middleware.is_file() else ""
        )
    }

    mounted: dict[str, set[str]] = {}
    for match in _ROUTE_CALL.finditer(lib):
        depth, i = 1, match.end()
        while i < len(lib) and depth:
            depth += 1 if lib[i] == "(" else -1 if lib[i] == ")" else 0
            i += 1
        methods = {m.group(1).upper() for m in _METHOD_CALL.finditer(lib[match.end() : i - 1])}
        literal = match.group("lit")
        if literal is None:
            literal = consts.get(match.group("const"))
            if literal is None:
                # A path this parse cannot resolve is reported rather than
                # skipped: silently dropping it would turn every claim about
                # that route into an unexplained "no such path".
                fail(
                    f"lib.rs mounts a route at `{match.group('const')}`, which this "
                    f"gate cannot resolve — teach it the constant or inline the path"
                )
                continue
        if literal.startswith("/api"):
            literal = literal[len("/api") :]
        mounted.setdefault(route_shape("/api" + literal), set()).update(methods)
    return mounted


def check_route_claims() -> int:
    """Grade every route a handler doc comment claims. Returns claims graded."""
    mounted = mounted_routes()
    if len(mounted) < 20:
        fail(
            f"only {len(mounted)} mounted routes found in lib.rs — this gate's parse "
            f"of `.route(...)` has stopped working"
        )
        return 0

    core_src = ROOT / "crates" / "core" / "src"
    handlers = core_src / "handlers"
    # `handlers.rs` is the module file and carries claims of its own; scanning
    # only the directory beside it grades 80 of the 99 and calls that all of
    # them, which is how a check reports a clean result it never measured.
    files = sorted(handlers.rglob("*.rs")) if handlers.is_dir() else []
    if (core_src / "handlers.rs").is_file():
        files.append(core_src / "handlers.rs")
    graded = 0
    for path in files:
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            for pattern in _ROUTE_CLAIMS:
                match = pattern.match(line)
                if not match:
                    continue
                graded += 1
                method = match.group("method")
                claimed = match.group("path").rstrip(".,")
                shape = route_shape(claimed)
                where = f"{rel(path)}:{number}"
                if shape not in mounted:
                    fail(f"{where}: documents `{method} {claimed}`, which is not mounted")
                elif method not in mounted[shape]:
                    served = "/".join(sorted(mounted[shape]))
                    fail(f"{where}: documents `{method} {claimed}`, mounted as {served}")
                break
    if not graded:
        fail("no route claims found in crates/core/src/handlers/ — update this script")
    return graded


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
        # Graded against the *display* spelling, not the tag's. These lines are
        # prose a reader sees, and the project shows `0.6.9a1` where the tag
        # says `v0.6.9-a.1`. Comparing against the tag would force the internal
        # spelling into the documentation; comparing against either would let
        # the two drift apart unnoticed, which is the state this gate exists to
        # prevent. For a final release and for the older long spelling the two
        # are the same string, so nothing already written moves.
        expected = expected_claim(versions[key])
        if stated.lstrip("v") != expected:
            fail(
                f"{name}: `{stated}` is marked {marker}, "
                f"but that is {expected}"
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


def expected_claim(tag_value: str) -> str:
    """The spelling a marked version claim must use, given the tag it names.

    A function rather than an expression at the comparison site so the rule can
    be asserted: with only long-spelling tags in the repository today, the
    display mapping is invisible at that site — `0.6.8-beta.7` maps to itself —
    and a change that dropped it would pass every check until the first short
    tag existed, months from now.
    """
    return to_display(tag_value.lstrip("v"))


def selftest() -> None:
    """Check the rules this file enforces that its own inputs cannot show yet."""
    assert expected_claim("v0.6.9-a.1") == "0.6.9a1", "claims must use the display spelling"
    assert expected_claim("0.6.9-b.7") == "0.6.9b7"
    # Unchanged for a final and for the older long spelling, so nothing already
    # written in the documentation moves.
    assert expected_claim("v0.6.8") == "0.6.8"
    assert expected_claim("v0.6.8-beta.7") == "0.6.8-beta.7"
    # Both spellings must be recognised as claims. A claim the pattern misses
    # is not one that fails — it is one that stops being graded.
    for line in (
        "0.6.9a1 <!-- docs-facts: latest-prerelease -->",
        "0.6.8-beta.7 <!-- docs-facts: latest-prerelease -->",
        "v0.6.8 <!-- docs-facts: latest-release -->",
    ):
        assert MARKED_VERSION.search(line), line
    # Placeholder spelling must not decide the answer, and a documented query
    # string must not become a different path — the two rules that, missing,
    # made a first cut of this check report nine findings where there were none.
    assert route_shape("/api/commands/:approval_id/approve") == route_shape(
        "/api/commands/{id}/approve"
    )
    assert route_shape("/api/cron/jobs[?agent_id=X]") == route_shape("/api/cron/jobs")
    assert route_shape("/api/modules/{id}/assets/*path") == route_shape(
        "/api/modules/{id}/assets/{*path}"
    )
    # But a different path must still be different, or the rule above would
    # make every claim agree with everything.
    assert route_shape("/api/modules") != route_shape("/api/modules/{id}")
    assert route_shape("/api/a/{}/b") != route_shape("/api/a/b/{}")
    print("selftest: OK")


def check_release_title(version: str | None) -> None:
    """The changelog section for the current version must declare a title.

    The release workflow composes the GitHub Release title from that
    declaration and refuses to publish without it. Checked here as well so the
    requirement lands on the pull request that bumps the version, rather than
    on the person cutting the release — by then the fix is a commit on top of a
    tag that has already frozen the tree.

    Skipped when the changelog has no section for the version yet: between a
    release and the next bump that is the normal state, and failing on it would
    make the gate red for a condition nobody can act on.
    """
    if version is None:
        return
    changelog = ROOT / "docs" / "CHANGELOG.md"
    if not changelog.is_file():
        fail("docs/CHANGELOG.md is missing — release titles are declared there")
        return
    text = changelog.read_text(encoding="utf-8")
    if not re.search(rf"^## \[{re.escape(version)}\]", text, re.M):
        return
    sys.path.insert(0, str(ROOT / "scripts"))
    try:
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "release_title", ROOT / "scripts" / "release-title.py"
        )
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        module.compose(text, version)
    except Exception as e:  # noqa: BLE001 - the message is the point
        fail(f"docs/CHANGELOG.md: {e}")


def main() -> int:
    if "--selftest" in sys.argv:
        selftest()
        return 0
    counts = measured_test_counts()
    cross_check_against_ratchet(counts)
    versions = measured_versions()
    check_release_title(versions.get("current"))
    env_defaults, env_seen = measured_env()
    routes_graded = check_route_claims()

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
        f"{versions.get('current', '?')}, {len(env_defaults)} env defaults, "
        f"{routes_graded} route claims)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
