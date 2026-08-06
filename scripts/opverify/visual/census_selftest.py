"""Self-test of the census walk (no VM, no CDP). Run:
``python -m scripts.opverify.visual.census_selftest`` (exit 0 = passed).

Covers: the walk visits every surface and records what each shows; a modal
surface is closed afterwards; an unreachable surface is reported instead of
losing the rest of the walk; and the safety predicate refuses a surface that
would navigate by clicking something that acts.

The safety check is exercised as a pure predicate and through `Surface`
construction — never by running a walk against a stub that would "click" a
quit button. A guard test that goes through the acting path is a guard test
that performs the act when the guard breaks.
"""

from __future__ import annotations

import sys

from . import census as C
from .journey import TargetSpec


class _StubTarget:
    def __init__(self, text, role="button"):
        self.text, self.role = text, role
        self.x, self.y = 10, 20
        self.enabled = True


class _StubTargeter:
    """Screen contents keyed by which surface the walk has navigated to."""

    def __init__(self, screens, unreachable=()):
        self._screens = screens
        self._unreachable = set(unreachable)
        self.here = "main"
        self.clicks = []

    def affordances(self):
        return [_StubTarget(t) for t in self._screens.get(self.here, [])]

    def find(self, contains, *, nth=0, require_enabled=False, exact=False):
        wanted = (contains,) if isinstance(contains, str) else tuple(contains)
        for w in wanted:
            key = w.lower()
            if key in self._unreachable:
                raise LookupError(f"no target whose text contains {w!r}")
            if key in self._screens:
                self.here = key
                return _StubTarget(w)
        raise LookupError(f"no target for {wanted!r}")


class _StubActuator:
    def __init__(self):
        self.sent = []

    def send(self, action):
        self.sent.append(action)


def _surfaces():
    return [
        C.Surface("main"),
        C.Surface("agents", TargetSpec(contains=("agents",))),
        C.Surface("settings", TargetSpec(contains=("settings",)), close_key="esc"),
    ]


def scenario_walk_records_each_surface() -> None:
    targeter = _StubTargeter(
        {
            "main": ["Cloto Assistant", "1 / 1 Active"],
            "agents": ["New agent", "Cloto Assistant"],
            "settings": ["Language", "Health"],
        }
    )
    census, unreached = C.take_census(
        targeter, _StubActuator(), _surfaces(), language="en", app_version="0.6.8", settle=0
    )
    assert unreached == [], unreached
    assert sorted(census.surfaces) == ["agents", "main", "settings"], census.surfaces

    # The same control on two surfaces stays two affordances.
    names = sorted(a.name for a in census.ids())
    assert names.count("cloto assistant") == 2, names
    # And the counter folded to one identity, not one per value.
    assert "# / # active" in names, names


def scenario_modal_surface_is_closed() -> None:
    """Without the close, the next surface's target sits behind an overlay."""
    actuator = _StubActuator()
    targeter = _StubTargeter({"main": ["a"], "agents": ["b"], "settings": ["c"]})
    C.take_census(targeter, actuator, _surfaces(), language="en", app_version="x", settle=0)
    keys = [a.key for a in actuator.sent if a.kind == "key"]
    assert keys == ["esc"], keys


def scenario_unreachable_surface_is_reported_not_fatal() -> None:
    """A denominator quietly short by a whole screen reads as better coverage
    than the suite has, so the walk names what it missed and keeps going."""
    targeter = _StubTargeter(
        {"main": ["a"], "settings": ["c"]}, unreachable={"agents"}
    )
    census, unreached = C.take_census(
        targeter, _StubActuator(), _surfaces(), language="en", app_version="x", settle=0
    )
    assert len(unreached) == 1 and unreached[0].startswith("agents:"), unreached
    # The rest of the walk still happened.
    assert sorted(census.surfaces) == ["main", "settings"], census.surfaces


def scenario_census_refuses_to_navigate_by_acting() -> None:
    """The sidebar's quit button is two items from the ones the walk wants, and
    the settings modal holds a Danger Zone. A measuring instrument that can
    uninstall the thing it measures is not one."""
    assert C.is_safe_nav(["エージェント", "agents"]) is True
    assert C.is_safe_nav(["設定", "settings"]) is True

    for bad in (["終了"], ["Quit"], ["アンインストール"], ["Delete agent"], ["電源"]):
        assert C.is_safe_nav(bad) is False, bad

    # One dangerous alternative condemns the declaration: the targeter matches
    # alternatives case-insensitively and takes the first hit, so a safe
    # Japanese label paired with a destructive English one still clicks the
    # destructive control when the app runs in English.
    assert C.is_safe_nav(["メモリ", "Reset memory"]) is False

    try:
        C.Surface("boom", TargetSpec(contains=("終了", "quit")))
    except C.UnsafeNavigation:
        pass
    else:
        raise AssertionError("a surface navigated by clicking an action")


def scenario_committed_surfaces_are_all_safe() -> None:
    """The shipped list is the thing that actually runs; assert it directly."""
    assert C.SURFACES[0].open_target is None, "the walk must start where the app is"
    for s in C.SURFACES:
        if s.open_target is None:
            continue
        contains = s.open_target.contains
        alts = (contains,) if isinstance(contains, str) else tuple(contains)
        assert C.is_safe_nav(alts), (s.name, alts)
    # Modals must declare how to leave, or every later surface is censused
    # through an overlay.
    assert C.SURFACES[-1].close_key == "esc", C.SURFACES[-1]


def main() -> int:
    scenarios = [
        scenario_walk_records_each_surface,
        scenario_modal_surface_is_closed,
        scenario_unreachable_surface_is_reported_not_fatal,
        scenario_census_refuses_to_navigate_by_acting,
        scenario_committed_surfaces_are_all_safe,
    ]
    for sc in scenarios:
        sc()
        print(f"  ok  {sc.__name__}")
    print(f"census selftest: {len(scenarios)}/{len(scenarios)} scenarios passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
