#!/usr/bin/env python3
"""The one mapping between the version a machine reads and the version a person reads.

The internal version is semver, and cannot be anything else: Cargo parses it,
Tauri parses it, the updater compares it, and the installer builds a download
URL and an asset filename out of the tag. A pre-release there is spelled
`0.6.9-a.1`, with the number as its own identifier so that ten sorts above nine.

The version a person reads has no parser behind it, and there it is spelled the
way the project's Python packages spell it: `0.6.9a1`. Same stage, same number,
one fewer punctuation mark.

    0.6.9-a.1   ->  0.6.9a1
    0.6.9-b.7   ->  0.6.9b7
    0.6.9-rc.1  ->  0.6.9rc1

Everything else passes through unchanged, and that is deliberate rather than
lazy:

- A final release (`0.6.8`) is spelled the same in both worlds.
- The long spelling (`0.6.8-beta.7`) is left alone because those releases were
  *published* under that name. Rewriting them here would make the documentation
  disagree with the tags, the assets and the release pages that already exist —
  a version's name is a fact about the past, not a style choice.

So the mapping applies from the 0.6.9 line onward, which is exactly where the
short spelling starts.

Usage:
    version_display.py --to-display 0.6.9-a.1     # 0.6.9a1
    version_display.py --to-semver 0.6.9a1        # 0.6.9-a.1
    version_display.py --selftest
"""

from __future__ import annotations

import argparse
import re
import sys

# The stages the short spelling uses. The alternation lists `rc` first out of
# habit, not necessity: both patterns are anchored and the stage is followed by
# a required `.` or digits, so `a|b|rc` matches exactly the same strings —
# checked by mutation rather than assumed, because the opposite is a common and
# plausible-sounding belief about alternation.
STAGES = ("rc", "a", "b")

_SEMVER_SHORT = re.compile(
    r"^(?P<core>\d+\.\d+\.\d+)-(?P<stage>rc|a|b)\.(?P<num>\d+)$"
)
_DISPLAY_SHORT = re.compile(
    r"^(?P<core>\d+\.\d+\.\d+)(?P<stage>rc|a|b)(?P<num>\d+)$"
)


def to_display(version: str) -> str:
    """Spell `version` the way a reader sees it. Unmapped forms pass through."""
    m = _SEMVER_SHORT.match(version)
    if not m:
        return version
    return f"{m['core']}{m['stage']}{m['num']}"


def to_semver(version: str) -> str:
    """Spell `version` the way a parser needs it. Unmapped forms pass through."""
    m = _DISPLAY_SHORT.match(version)
    if not m:
        return version
    return f"{m['core']}-{m['stage']}.{m['num']}"


def selftest() -> None:
    assert to_display("0.6.9-a.1") == "0.6.9a1"
    assert to_display("0.6.9-b.7") == "0.6.9b7"
    assert to_display("0.6.9-rc.1") == "0.6.9rc1"
    assert to_semver("0.6.9a1") == "0.6.9-a.1"
    assert to_semver("0.6.9b7") == "0.6.9-b.7"
    assert to_semver("0.6.9rc1") == "0.6.9-rc.1"

    # Pass-through, each for its own reason (see the module docstring).
    for unchanged in (
        "0.6.8",  # a final is spelled the same in both worlds
        "0.6.8-beta.7",  # published under this name; the past is not restyled
        "0.6.8-alpha.1",
        "1.0.0",
    ):
        assert to_display(unchanged) == unchanged, unchanged
        assert to_semver(unchanged) == unchanged, unchanged

    # `rc` is the one stage both spellings write the same way, so a version
    # like `0.6.8-rc.1` is indistinguishable by shape from a 0.6.9-line one and
    # maps rather than passing through. That would restyle a published name if
    # any release had used it — measured across all tags in the repository:
    # twenty `alpha`, twenty-three `beta`, and no `rc` at all. So the shared
    # spelling costs nothing here, and this assertion records the fact rather
    # than the hope.
    assert to_display("0.6.8-rc.1") == "0.6.8rc1"

    # A round trip must return the same string in both directions, or the two
    # spellings are not two spellings of one thing. Generated rather than
    # listed, so a stage added to STAGES is covered without anyone remembering
    # to add a case — including the double-digit numbers that motivated the
    # dotted form in the first place.
    for stage in STAGES:
        for num in (1, 2, 9, 10, 11, 100):
            semver = f"0.6.9-{stage}.{num}"
            display = f"0.6.9{stage}{num}"
            assert to_display(semver) == display, semver
            assert to_semver(display) == semver, display
            assert to_semver(to_display(semver)) == semver, semver
            assert to_display(to_semver(display)) == display, display

    # Distinct versions must not collapse onto one display string: a mapping
    # that is not injective would make two releases indistinguishable to a
    # reader, which is worse than showing the punctuation.
    versions = [f"0.6.9-{s}.{n}" for s in STAGES for n in (1, 2, 10)] + [
        "0.6.9",
        "0.6.8-beta.7",
        "0.6.8-alpha.1",
    ]
    displays = [to_display(v) for v in versions]
    assert len(set(displays)) == len(displays), "two versions share a display form"

    # `rc` must not be read as an `r` followed by junk, nor `b` swallow `beta`.
    assert to_display("0.6.9-rc.2") == "0.6.9rc2"
    assert to_semver("0.6.9rc2") == "0.6.9-rc.2"
    assert to_display("0.6.9-beta.2") == "0.6.9-beta.2"

    # Shapes that are neither: refusing to guess is the point.
    for junk in ("", "not a version", "0.6.9-", "0.6.9-a", "0.6.9-a.", "v0.6.9-a.1"):
        assert to_display(junk) == junk, junk
        assert to_semver(junk) == junk, junk

    print("selftest: OK")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    g = ap.add_mutually_exclusive_group()
    g.add_argument("--to-display", metavar="VERSION")
    g.add_argument("--to-semver", metavar="VERSION")
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()

    if args.selftest:
        selftest()
        return 0
    if args.to_display:
        print(to_display(args.to_display))
        return 0
    if args.to_semver:
        print(to_semver(args.to_semver))
        return 0
    ap.error("give --to-display, --to-semver or --selftest")
    return 2


if __name__ == "__main__":
    sys.exit(main())
