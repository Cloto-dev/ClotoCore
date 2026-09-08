#!/usr/bin/env python3
"""Compose the GitHub Release title for a version.

The title is `<version> — <one sentence about what the version did>`, with no
`v` prefix and the version in its display spelling (`0.6.9a1`, not
`0.6.9-a.1`) — the form the project's Python packages use, so a reader moving
between the two repositories sees one convention instead of two. The mapping
lives in `version_display.py`; nothing about the internal version changes.

The sentence cannot be derived from the changelog prose. A good release title
is written, not extracted: the first sentence of a changelog entry explains the
release to someone already reading it, while the title has to say what changed
to someone scrolling a list. So the sentence is declared, in the changelog
section it belongs to:

    ## [0.6.9a1] — 2026-09-08
    <!-- release-title: an agent gets files it always reads -->

Declaring it there rather than in a file of its own is what keeps it honest.
A standalone file would still hold the previous release's sentence on the day
someone forgot to update it, and would title the new release with it — silently,
because nothing would be wrong with the file. Keyed by version inside the
section it describes, a stale sentence is not reachable: a release with no
section of its own has no title, and this script refuses rather than inventing
one.

Usage:
    release-title.py --version 0.6.9-a.1        # prints the title
    release-title.py --selftest                 # checks the rules above
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from version_display import to_display  # noqa: E402

# Em dash, the separator the sibling project uses. Written as an escape so the
# byte cannot be mistaken for a hyphen when this file is read or edited.
SEPARATOR = "\u2014"

MARKER_RE = re.compile(r"<!--\s*release-title:\s*(?P<sentence>.*?)\s*-->", re.S)

# Long enough for a sentence that names two changes and their consequence;
# short enough to read in a release list. A title over this is refused rather
# than trimmed: a trimmed title is a sentence that stops mid-clause, and the
# person who wrote it is the one who should decide what to drop.
MAX_TITLE_CHARS = 160


class TitleError(Exception):
    """The changelog cannot supply a title for this version."""


def section_of(changelog: str, version: str) -> str:
    """Return the changelog section for `version`, without its heading.

    The heading form is `## [<version>] — <date>`; the section runs to the next
    `## ` heading or the end of the file.
    """
    start = None
    lines = changelog.splitlines()
    heading = re.compile(rf"^## \[{re.escape(version)}\]")
    for i, line in enumerate(lines):
        if heading.match(line):
            start = i + 1
            break
    if start is None:
        raise TitleError(
            f"docs/CHANGELOG.md has no section for {version!r} "
            f"(expected a heading '## [{version}] — <date>')"
        )
    end = len(lines)
    for i in range(start, len(lines)):
        if lines[i].startswith("## "):
            end = i
            break
    return "\n".join(lines[start:end])


def sentence_of(section: str, version: str) -> str:
    """Return the declared title sentence from one changelog section."""
    found = MARKER_RE.findall(section)
    if not found:
        raise TitleError(
            f"the {version} changelog section declares no release title — add "
            f"'<!-- release-title: ... -->' directly under its heading"
        )
    if len(found) > 1:
        # Two markers is not a merge to resolve automatically: picking either
        # one would publish a title nobody chose.
        raise TitleError(
            f"the {version} changelog section declares {len(found)} release "
            f"titles; it must declare exactly one"
        )
    sentence = found[0].strip()
    if not sentence:
        raise TitleError(f"the {version} release title is empty")
    if "\n" in sentence:
        raise TitleError(
            f"the {version} release title spans lines; a title is one line"
        )
    return sentence


def compose(changelog: str, version: str) -> str:
    """Compose the release title for `version`, or raise `TitleError`.

    `version` arrives in the internal spelling — it comes from the tag — and
    both the changelog heading and the title use the display one, so it is
    mapped once here and used in both places. A section written under the
    internal spelling is therefore not found, which is the intended failure:
    the heading is what a reader sees, and readers get one spelling.
    """
    shown = to_display(version)
    sentence = sentence_of(section_of(changelog, shown), shown)
    title = f"{shown} {SEPARATOR} {sentence}"
    if len(title) > MAX_TITLE_CHARS:
        raise TitleError(
            f"the {version} release title is {len(title)} characters, over the "
            f"{MAX_TITLE_CHARS} the release list can show; shorten the sentence"
        )
    return title


def selftest() -> None:
    good = (
        "# Changelog\n\n"
        "## [0.6.9a1] — 2026-09-08\n"
        "<!-- release-title: an agent gets files it always reads -->\n\n"
        "Lead paragraph.\n\n"
        "## [0.6.8] — 2026-09-07\n"
        "<!-- release-title: the line reaches the stable channel -->\n\n"
        "Older.\n"
    )

    # Called with the spelling the tag carries; answers in the spelling a
    # reader sees, and finds the section under that same one.
    assert (
        compose(good, "0.6.9-a.1")
        == "0.6.9a1 — an agent gets files it always reads"
    )
    # Idempotent: handed the display spelling it maps to itself, so a caller
    # that already converted is not punished for it.
    assert compose(good, "0.6.9a1") == compose(good, "0.6.9-a.1")
    # No `v`, and an em dash rather than a hyphen: both are the point of the
    # format, so both are asserted rather than left to the reader of the
    # f-string. The separator is checked in place, against the version it
    # follows — a bare "an em dash appears somewhere" would also pass on a
    # sentence that happens to contain one.
    assert not compose(good, "0.6.8").startswith("v")
    assert compose(good, "0.6.8").startswith("0.6.8 — ")
    assert not compose(good, "0.6.8").startswith("0.6.8 - ")

    # The section is found by version, so an older section's sentence cannot be
    # served for a newer release — the failure a standalone title file has.
    assert compose(good, "0.6.8").endswith("the line reaches the stable channel")

    def refuses(changelog: str, version: str, because: str) -> None:
        try:
            compose(changelog, version)
        except TitleError:
            return
        raise AssertionError(f"should have refused: {because}")

    refuses(good, "0.7.0", "no section for the version")
    refuses(
        "## [1.0.0] — 2026-01-01\n\nNo marker here.\n", "1.0.0", "no marker"
    )
    refuses(
        "## [1.0.0] — 2026-01-01\n<!-- release-title:  -->\n", "1.0.0", "empty title"
    )
    refuses(
        "## [1.0.0] — 2026-01-01\n<!-- release-title: one -->\n"
        "<!-- release-title: two -->\n",
        "1.0.0",
        "two markers",
    )
    refuses(
        "## [1.0.0] — 2026-01-01\n<!-- release-title: " + "x" * MAX_TITLE_CHARS + " -->\n",
        "1.0.0",
        "over the length ceiling",
    )
    refuses(
        "## [1.0.0] — 2026-01-01\n<!-- release-title: one\nline two -->\n",
        "1.0.0",
        "a title spanning lines",
    )

    # A heading in the internal spelling is not a heading a reader would see,
    # so it is not found — the failure that keeps one spelling in the file
    # rather than two that drift.
    refuses(
        "## [0.6.9-a.1] — 2026-09-08\n<!-- release-title: internal spelling -->\n",
        "0.6.9-a.1",
        "a heading written in the internal spelling",
    )

    # A marker in a *later* section must not be read for an earlier one: the
    # scan stops at the next heading.
    two = (
        "## [2.0.0] — 2026-02-01\n\nNo marker.\n\n"
        "## [1.0.0] — 2026-01-01\n<!-- release-title: the first one -->\n"
    )
    refuses(two, "2.0.0", "reading the next section's marker")

    print("selftest: OK")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--version", help="version to compose a title for")
    ap.add_argument(
        "--changelog",
        default="docs/CHANGELOG.md",
        help="path to the changelog (default: docs/CHANGELOG.md)",
    )
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()

    if args.selftest:
        selftest()
        return 0
    if not args.version:
        ap.error("--version is required unless --selftest")

    try:
        text = Path(args.changelog).read_text(encoding="utf-8")
    except OSError as e:
        print(f"::error::cannot read {args.changelog}: {e}", file=sys.stderr)
        return 1

    try:
        print(compose(text, args.version))
    except TitleError as e:
        print(f"::error::{e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
