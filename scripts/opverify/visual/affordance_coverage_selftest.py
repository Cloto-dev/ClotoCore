"""Self-test of the affordance coverage arithmetic (no VM, no CDP). Run:
``python -m scripts.opverify.visual.affordance_coverage_selftest`` (exit 0 = passed).

Covers: digit folding treats a counter's states as one affordance; the same
name on two surfaces stays two; declaration matching reproduces TargetSpec's
substring/exact semantics including the nested-name case that needed `exact`
in the first place; a declaration that matches nothing is reported rather than
silently counted as zero; census JSON round-trips; the percentage arithmetic.

The journey fixtures here are built from the real `journey` dataclasses rather
than hand-drawn stand-ins — a numerator derived from a shape the harness does
not actually write would be worth nothing.
"""

from __future__ import annotations

import sys

from . import affordance_coverage as AC
from . import journey as J


def _census() -> AC.Census:
    c = AC.Census(language="ja", app_version="0.6.8-beta.4")
    c.add(
        "main",
        [
            AC.AffordanceId.of("main", "button", "エージェント"),
            AC.AffordanceId.of("main", "button", "設定"),
            AC.AffordanceId.of("main", "button", "1 / 1 Active"),
        ],
    )
    c.add(
        "settings",
        [
            AC.AffordanceId.of("settings", "button", "設定"),
            AC.AffordanceId.of("settings", "button", "CLOTOCORE をアンインストール"),
            AC.AffordanceId.of("settings", "button", "アンインストール"),
        ],
    )
    return c


def scenario_digits_are_state_not_identity() -> None:
    """A counter in its every state is one affordance. Counting each state
    separately would grow the denominator every time the app was used, which
    makes the ratchet fall for reasons that have nothing to do with coverage."""
    a = AC.AffordanceId.of("main", "button", "1 / 1 Active")
    b = AC.AffordanceId.of("main", "button", "12 / 30 Active")
    assert a == b, (a, b)
    assert a.name == "# / # active", a.name

    # Folding must not merge things that differ by more than their numbers.
    assert AC.AffordanceId.of("main", "button", "3 メモリ") != AC.AffordanceId.of(
        "main", "button", "3 エージェント"
    )
    # Whitespace and case are noise; the words are not.
    assert AC.canon_name("  Save   Changes ") == AC.canon_name("save changes")


def scenario_same_name_on_two_surfaces_is_two_affordances() -> None:
    """Reaching a control is part of what a journey has to get right, so the
    "設定" button on the main window and the one inside the settings modal are
    not interchangeable."""
    census = _census()
    ids = census.ids()
    settings = sorted(a.surface for a in ids if a.name == "設定")
    assert settings == ["main", "settings"], settings
    assert len(ids) == 6, sorted(ids)


def scenario_declaration_matching_mirrors_targetspec() -> None:
    """Substring by default, whole-name under `exact` — the semantics the
    driver actually resolves with. The nested pair is why `exact` exists: the
    card "CLOTOCORE をアンインストール" contains the dialog's "アンインストール",
    and a run that hit the wrong one uninstalled from the wrong gate."""
    census = _census()

    loose = AC.Declaration("j", "s", ("アンインストール",))
    hits = sorted(a.name for a in census.ids() if loose.matches(a))
    assert hits == ["clotocore をアンインストール", "アンインストール"], hits

    strict = AC.Declaration("j", "s", ("アンインストール",), exact=True)
    hits = sorted(a.name for a in census.ids() if strict.matches(a))
    assert hits == ["アンインストール"], hits

    # Alternatives are locale variants: any one matching is a match.
    either = AC.Declaration("j", "s", ("Uninstall", "アンインストール"), exact=True)
    assert any(either.matches(a) for a in census.ids())


def scenario_declarations_come_from_real_journey_objects() -> None:
    """`declared_targets` reads the real Step/TargetSpec dataclasses, and steps
    that declare no target contribute nothing."""
    jr = J.Journey(
        name="settings-tour",
        steps=[
            J.Step(name="open", target=J.TargetSpec(contains="設定")),
            J.Step(name="look"),  # assertion only — declares nothing
            J.Step(
                name="purge",
                target=J.TargetSpec(contains=("アンインストール",), exact=True),
            ),
        ],
    )
    decls = AC.declared_targets(jr)
    assert [d.step for d in decls] == ["open", "purge"], decls
    assert decls[0].alternatives == ("設定",) and not decls[0].exact
    assert decls[1].exact is True


def scenario_unmatched_declaration_is_reported_not_zero() -> None:
    """A step whose target is nowhere in the census means the journey has gone
    stale or the census never visited that surface. Both are findings; folding
    them into "covered 0" would hide the difference between a suite that
    covers little and a suite that is measuring the wrong app."""
    census = _census()
    report = AC.coverage(
        census,
        [
            AC.Declaration("j", "open-settings", ("設定",)),
            AC.Declaration("j", "click-ghost", ("この文字列はどこにもない",)),
        ],
    )
    assert [d.step for d in report.unmatched_declarations] == ["click-ghost"], report.as_dict()
    # The matched one still counted, on both surfaces it appears.
    assert report.covered == 2, report.as_dict()


def scenario_coverage_arithmetic() -> None:
    census = _census()
    empty = AC.coverage(census, [])
    assert empty.total == 6 and empty.covered == 0 and empty.coverage_pct == 0.0
    assert len(empty.uncovered_ids) == 6

    partial = AC.coverage(census, [AC.Declaration("j", "s", ("エージェント",))])
    assert partial.covered == 1 and partial.total == 6
    assert partial.coverage_pct == 16.7, partial.coverage_pct
    assert all(a.name != "エージェント" for a in partial.uncovered_ids)

    # An empty census divides by zero unless guarded; it must read as 0%, not crash.
    bare = AC.Census(language="ja", app_version="x")
    assert AC.coverage(bare, []).coverage_pct == 0.0


def scenario_census_round_trips() -> None:
    census = _census()
    back = AC.Census.from_json(census.to_json())
    assert back.language == "ja" and back.app_version == "0.6.8-beta.4"
    assert back.ids() == census.ids()
    assert sorted(back.surfaces) == ["main", "settings"]

    # Re-censusing a surface must not double it — the walk may revisit a screen.
    census.add_targets(
        "main", [type("T", (), {"role": "button", "text": "エージェント"})()]
    )
    assert len(census.surfaces["main"]) == 3, census.surfaces["main"]


def scenario_ignore_is_explicit() -> None:
    """The ignore set shrinks the denominator, so it is opt-in and starts
    empty: an entry invented before a census exists to justify it would quietly
    flatter the number."""
    assert AC.DEFAULT_IGNORE == set(), AC.DEFAULT_IGNORE
    census = _census()
    assert len(census.ids()) == 6
    assert len(census.ids(ignore={("button", "設定")})) == 4


def main() -> int:
    scenarios = [
        scenario_digits_are_state_not_identity,
        scenario_same_name_on_two_surfaces_is_two_affordances,
        scenario_declaration_matching_mirrors_targetspec,
        scenario_declarations_come_from_real_journey_objects,
        scenario_unmatched_declaration_is_reported_not_zero,
        scenario_coverage_arithmetic,
        scenario_census_round_trips,
        scenario_ignore_is_explicit,
    ]
    for sc in scenarios:
        sc()
        print(f"  ok  {sc.__name__}")
    print(f"affordance-coverage selftest: {len(scenarios)}/{len(scenarios)} scenarios passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
