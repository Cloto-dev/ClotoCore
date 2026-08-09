"""How much of the GUI the frozen journeys actually touch.

The kernel tier has had a coverage ratchet since phase 1: the denominator is
parsed out of the route table, the numerator is what each catalog operation
declares it ``covers``, and a route nobody claims is reported (later,
enforced). The apex has had nothing. Its ledger rows carry
``coverage_pct: 0.0 / covered: 0 / total_routes: 0`` because there was no
denominator to divide by, so "how much of the app do these journeys exercise"
could only be answered by opinion — and a suite that cannot be counted is a
suite nobody can defend keeping.

This module supplies the arithmetic. Two halves, deliberately split by what
they need:

* **The denominator is a census** — the affordances actually enumerated on the
  running app, surface by surface, by :meth:`cdp.CdpTargeter.affordances`.
  There is no static source to parse: the whole premise of the apex is that
  what the DOM renders and what the source claims are different questions, and
  the one that matters to a user is the first. So the denominator costs a VM
  run and is committed as a baseline artifact.
* **The numerator needs no VM at all.** Journeys already declare *what* they
  act on rather than *where* it is (:class:`journey.TargetSpec`, adopted after
  hardcoded coordinates failed twice wearing a disguise), and they declare it
  in the same vocabulary the census records: visible text. So what the suite
  covers is derivable from the committed journey definitions.

The identity problem is the whole difficulty. A screen coordinate is true for
one scroll position; visible text moves with locale and with state. So an
affordance is identified by ``(surface, role, canonical name)``, where the
canonical name collapses whitespace, casefolds, and **folds runs of digits to
``#``** — the same move the kernel ratchet makes when it canonicalizes ``{id}``
to ``{}``, and for the same reason: "1 / 1 active" and "3 / 4 active" are one
affordance in two states, not two affordances, and counting them separately
would inflate the denominator every time the app was used.

A census is only comparable to another census of the same UI language and app
version, which is why it records both. VM 104 runs a Japanese UI; a census
taken there cannot be divided into an English one's numbers. (The ledger draws
the same line when it keys baselines by target/os/journey/assessor.)
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from typing import Dict, Iterable, List, Optional, Sequence, Set, Tuple

# Placeholder for a control that renders no accessible name at all. Kept
# distinctive so an uncovered report says plainly that the thing has no name,
# rather than showing a blank the reader has to interpret.
UNNAMED = "«unnamed»#"

# Runs of digits are state, not identity — see the module docstring.
_DIGITS = re.compile(r"\d+")
_SPACE = re.compile(r"\s+")


def canon_name(text: str) -> str:
    """The comparable form of a visible name.

    Order matters: fold digits before collapsing whitespace, so "1 / 1" and
    "12 / 30" reduce to the same "# / #" rather than differing by the width of
    the numbers.
    """
    return _SPACE.sub(" ", _DIGITS.sub("#", text or "")).strip().casefold()


@dataclass(frozen=True, order=True)
class AffordanceId:
    """One thing a user could act on, identified so it survives a re-census.

    ``surface`` is which screen it was found on. The same control on two
    screens is two affordances: reaching it is part of what a journey has to
    get right, and a suite that only ever opens one of the two screens has not
    covered both.
    """

    surface: str
    role: str
    name: str

    @classmethod
    def of(cls, surface: str, role: str, text: str) -> "AffordanceId":
        return cls(surface=surface, role=(role or "").strip().casefold(), name=canon_name(text))

    def as_dict(self) -> dict:
        return {"surface": self.surface, "role": self.role, "name": self.name}


# Affordances that are real but are not the app's to be judged on. Deliberately
# EMPTY: the honest membership of this set can only be read off a census that
# has actually been taken, and inventing plausible-looking entries now would
# quietly shrink the denominator by guesswork. Add entries with the census row
# that justifies each one.
DEFAULT_IGNORE: Set[Tuple[str, str]] = set()


@dataclass
class Census:
    """Every affordance enumerated on a build, surface by surface.

    Not a set of names: the same name on two surfaces is two entries, and the
    per-surface breakdown is what makes an uncovered report actionable ("the
    suite never opens CRON") instead of a flat list.
    """

    language: str
    app_version: str
    # What the app was showing when the walk began. `main` is defined as
    # "wherever the app already is", so the denominator moves with the start
    # state and two censuses taken from different ones are not comparable —
    # measured 2026-08-08, when a re-take put main at 24 against a baseline's
    # 16 because an agent-creation form happened to be open. The totals looked
    # like coverage had changed; nothing had. An artifact that does not say
    # where it started cannot be checked for this, so the field is required at
    # the point of capture (census.py --start-state) rather than optional here.
    start_state: str = ""
    surfaces: Dict[str, List[AffordanceId]] = field(default_factory=dict)

    def add(self, surface: str, affordances: Iterable[AffordanceId]) -> None:
        bucket = self.surfaces.setdefault(surface, [])
        seen = set(bucket)
        for a in affordances:
            if a not in seen:  # a re-census of one surface must not double it
                seen.add(a)
                bucket.append(a)

    def add_targets(self, surface: str, targets: Iterable) -> None:
        """Ingest :class:`cdp.Target` rows (or anything with .role/.text).

        Controls with no accessible name are numbered per role, in enumeration
        order, instead of all collapsing onto one nameless identity. They have
        to be: the main window carries seven of them — history back/forward,
        help, minimize, maximize, close, and an in-page back arrow — and one
        name for all seven understated the denominator by six (measured
        2026-08-06, and the reason bug-502 was filed).

        The ordinal is DOM order, which is as stable as an unnamed control can
        be. It is deliberately not position: a coordinate moves with the
        window, and identity that moves is not identity. A journey can never
        match one of these anyway — journeys declare targets by visible text —
        so they stay uncovered, which is the truthful reading rather than an
        inflated score.
        """
        ids: List[AffordanceId] = []
        anonymous: Dict[str, int] = {}
        for t in targets:
            a = AffordanceId.of(surface, t.role, t.text)
            if not a.name:
                anonymous[a.role] = anonymous.get(a.role, 0) + 1
                a = AffordanceId(
                    surface=surface, role=a.role, name=f"{UNNAMED}{anonymous[a.role]}"
                )
            ids.append(a)
        self.add(surface, ids)

    def ids(self, ignore: Optional[Set[Tuple[str, str]]] = None) -> Set[AffordanceId]:
        drop = DEFAULT_IGNORE if ignore is None else ignore
        return {
            a
            for bucket in self.surfaces.values()
            for a in bucket
            if (a.role, a.name) not in drop
        }

    def as_dict(self) -> dict:
        return {
            "language": self.language,
            "app_version": self.app_version,
            "start_state": self.start_state,
            "surfaces": {
                s: [a.as_dict() for a in bucket] for s, bucket in self.surfaces.items()
            },
        }

    def to_json(self) -> str:
        return json.dumps(self.as_dict(), ensure_ascii=False, indent=2, sort_keys=True)

    @classmethod
    def from_dict(cls, d: dict) -> "Census":
        # Absent on artifacts taken before the field existed. They read back as
        # "" — unknown, not "the same start state as yours".
        c = cls(
            language=d["language"],
            app_version=d["app_version"],
            start_state=d.get("start_state", ""),
        )
        for surface, rows in d.get("surfaces", {}).items():
            c.surfaces[surface] = [
                AffordanceId(surface=r["surface"], role=r["role"], name=r["name"])
                for r in rows
            ]
        return c

    @classmethod
    def from_json(cls, text: str) -> "Census":
        return cls.from_dict(json.loads(text))


@dataclass(frozen=True)
class Declaration:
    """What one journey step says it acts on, in the journey's own words.

    Mirrors :class:`journey.TargetSpec` matching: ``alternatives`` are the
    locale variants a step accepts, and ``exact`` selects whole-name matching
    (needed where one control's name is a prefix of another's).
    """

    journey: str
    step: str
    alternatives: Tuple[str, ...]
    exact: bool = False

    def matches(self, affordance: AffordanceId) -> bool:
        for alt in self.alternatives:
            want = canon_name(alt)
            if not want:
                continue
            if affordance.name == want or (not self.exact and want in affordance.name):
                return True
        return False


def declared_targets(journey) -> List[Declaration]:
    """Read a built journey's target declarations.

    Steps without a target declare nothing — they are positioning, waiting or
    pure assertion — and contribute no coverage.
    """
    out: List[Declaration] = []
    for step in getattr(journey, "steps", []):
        spec = getattr(step, "target", None)
        if spec is None:
            continue
        contains = getattr(spec, "contains", None)
        alts = (contains,) if isinstance(contains, str) else tuple(contains or ())
        out.append(
            Declaration(
                journey=getattr(journey, "name", "?"),
                step=getattr(step, "name", "?"),
                alternatives=tuple(str(a) for a in alts),
                exact=bool(getattr(spec, "exact", False)),
            )
        )
    return out


@dataclass
class CoverageReport:
    total: int
    covered_ids: List[AffordanceId]
    uncovered_ids: List[AffordanceId]
    # Declarations that matched nothing in the census. This is a finding, not a
    # zero: either a journey has gone stale against the UI, or the census never
    # visited the surface the step acts on. Reporting it as "0 covered" would
    # hide both.
    unmatched_declarations: List[Declaration]

    @property
    def covered(self) -> int:
        return len(self.covered_ids)

    @property
    def coverage_pct(self) -> float:
        if not self.total:
            return 0.0
        return round(100.0 * self.covered / self.total, 1)

    def as_dict(self) -> dict:
        return {
            "coverage_pct": self.coverage_pct,
            "covered": self.covered,
            "total": self.total,
            "uncovered": [a.as_dict() for a in self.uncovered_ids],
            "unmatched_declarations": [
                {"journey": d.journey, "step": d.step, "alternatives": list(d.alternatives)}
                for d in self.unmatched_declarations
            ],
        }


def coverage(
    census: Census,
    declarations: Sequence[Declaration],
    ignore: Optional[Set[Tuple[str, str]]] = None,
) -> CoverageReport:
    """Cross the census against what the journeys declare they act on."""
    universe = sorted(census.ids(ignore))
    covered: Set[AffordanceId] = set()
    unmatched: List[Declaration] = []
    for decl in declarations:
        hits = [a for a in universe if decl.matches(a)]
        if hits:
            covered.update(hits)
        else:
            unmatched.append(decl)
    return CoverageReport(
        total=len(universe),
        covered_ids=sorted(covered),
        uncovered_ids=[a for a in universe if a not in covered],
        unmatched_declarations=unmatched,
    )
