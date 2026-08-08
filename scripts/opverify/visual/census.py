"""Take the affordance census — the denominator the apex ratchet divides by.

:mod:`affordance_coverage` supplies the arithmetic and derives the numerator
from what the journeys declare. The denominator cannot be derived from
anything: it is what the app actually renders, so somebody has to walk the
surfaces and look. That is this module.

The walk is deliberately dull. It clicks only the navigation targets named in
:data:`SURFACES` — a small committed list — enumerates what is on screen, and
moves on. It asserts nothing, so it cannot fail a journey; it changes no data,
though it does leave the app on whatever screen it visited last.

**It must never click anything that acts.** The sidebar's quit button sits two
items below the ones the walk wants, the settings modal contains a Danger Zone,
and a census that "explored" would eventually find them. So a surface's
navigation target is checked against :func:`is_safe_nav` when the surface is
constructed — the census is a measuring instrument, and a measuring instrument
that can uninstall the thing it measures is not one.

Run (needs the VM, the CDP port open, and OPV_API_KEY):
``python -m scripts.opverify.visual.census --out qa/opverify/affordance-census.json``
"""

from __future__ import annotations

import argparse
import sys
import time
from dataclasses import dataclass
from typing import List, Optional, Sequence, Tuple

from .affordance_coverage import Census
from .cdp import captured_size, space_mismatch, to_screen
from .interfaces import click, move, press_key, scroll
from .journey import TargetSpec

# Words that name an action rather than a destination. Matched against a
# surface's declared navigation target, not against what is on screen — the
# census reads the whole screen but only ever clicks what SURFACES names.
_ACTS_NOT_NAVIGATES: Tuple[str, ...] = (
    "終了",
    "quit",
    "exit",
    "アンインストール",
    "uninstall",
    "削除",
    "delete",
    "リセット",
    "reset",
    "再生成",
    "regenerate",
    "実行",
    "execute",
    "purge",
    "無効化",
    "invalidate",
    "電源",
    "power",
)


def is_safe_nav(alternatives: Sequence[str]) -> bool:
    """True when every alternative names a place, not a deed.

    Any one alternative being dangerous condemns the whole declaration: the
    targeter matches alternatives case-insensitively and takes the first hit,
    so a safe-looking Japanese label paired with a destructive English one is
    still a click on the destructive control when the app runs in English.
    """
    for alt in alternatives:
        low = (alt or "").strip().lower()
        if not low:
            continue
        if any(bad in low for bad in _ACTS_NOT_NAVIGATES):
            return False
    return True


class UnsafeNavigation(ValueError):
    """A surface tried to navigate by clicking something that acts."""


@dataclass
class Surface:
    """One screen worth censusing, and how to get to it.

    `open_target` is None for the screen the app is already on when the walk
    starts. `close_key` returns from a modal — without it the next surface's
    navigation target is behind an overlay and resolves to something the click
    cannot reach.

    `reviewed_safe` is the escape hatch for a control whose *name* reads as an
    action but whose behaviour is navigation. It is deliberately awkward: it
    takes a written reason, it is per-surface, and it never widens
    :data:`_ACTS_NOT_NAVIGATES` — because relaxing the word list to admit one
    reviewed control would silently admit every unreviewed one that shares the
    word.
    """

    name: str
    open_target: Optional[TargetSpec] = None
    close_key: Optional[str] = None
    reviewed_safe: str = ""
    # A control inside the scrolling pane that holds `open_target`, used to put
    # the pointer somewhere the wheel will move the right thing. The wheel acts
    # where the cursor is, so without this the census would scroll the window
    # behind the modal and conclude the entry point is unreachable.
    scroll_over: Optional[TargetSpec] = None

    def __post_init__(self):
        if self.open_target is None:
            return
        contains = self.open_target.contains
        alts = (contains,) if isinstance(contains, str) else tuple(contains or ())
        if is_safe_nav(alts):
            return
        if self.reviewed_safe:
            return
        raise UnsafeNavigation(
            f"surface {self.name!r} navigates by clicking {alts!r}, which names "
            "an action. The census may only click its way between screens. If "
            "this control navigates despite its name, say why in reviewed_safe."
        )


# VM 104 runs the Japanese pack while the locale files are authored in English,
# so every surface carries both spellings — the same reason journeys do.
SURFACES: List[Surface] = [
    Surface("main"),  # the walk starts wherever the app already is
    Surface("agents", TargetSpec(contains=("エージェント", "agents"))),
    Surface("mcp", TargetSpec(contains=("MCP",))),
    Surface("cron", TargetSpec(contains=("CRON", "cron"))),
    Surface("memory", TargetSpec(contains=("メモリ", "memory"))),
    Surface("system", TargetSpec(contains=("システム", "system"))),
    # The settings modal and its nested views come last, and only the last of
    # them closes: an `esc` in the middle would drop the walk back to the main
    # window and every later view would be censused through an overlay.
    Surface("settings", TargetSpec(contains=("設定", "settings"))),
    Surface("settings-health", TargetSpec(contains=("ヘルス", "health"))),
    # The Danger Zone. Five of danger-zone-purge's seven declarations act in
    # here, and while it was outside the census they matched nothing — the
    # denominator was short by a whole view and the suite's score understated
    # what it covers.
    #
    # The walk STOPS at this view. It enumerates the scope checkboxes, the
    # admin-key field and the uninstall button, and touches none of them: the
    # next step widens the scope, and the one after that is the real thing. The
    # confirm dialog's button can therefore never be censused — it exists only
    # once an uninstall has been initiated — so one declaration stays
    # permanently unmatched, and that is the honest state rather than a gap to
    # paper over.
    Surface(
        "settings-danger-zone",
        TargetSpec(contains=("削除される対象を確認", "review what would be removed")),
        close_key="esc",
        # The button sits below the health pane's fold; the wheel has to act
        # inside that pane, so the pointer goes to a control known to be in it.
        scroll_over=TargetSpec(contains=("スキャン", "scan")),
        reviewed_safe=(
            "Named for what the uninstall would remove, but it performs no "
            "removal: it opens the Danger Zone's read-only enumeration (the "
            "plan endpoint is a GET, documented as the first gate precisely "
            "so the dashboard can show what an uninstall would take before "
            "one is started). Reviewed 2026-08-06."
        ),
    ),
]


def _backdrop_point(targeter) -> Optional[Tuple[int, int]]:
    """A point inside the window that no affordance occupies.

    A modal in this app closes when its backdrop is clicked — the apex runbook
    records that as the way three runs accidentally dismissed one. Escape does
    not close it (measured 2026-08-06: the actuator reports the key sent and
    the frame is unchanged) and its close button carries no accessible name, so
    the backdrop is the only handle a perceptual harness has.

    "Where there is nothing" is computed, not guessed: candidates are rejected
    if they fall inside any enumerated affordance's rect, so the click cannot
    land on a control. Returns None when no candidate is clear, and the caller
    then leaves the screen alone rather than clicking blind.
    """
    f = getattr(targeter, "last_frame", {}) or {}
    if not f:
        return None
    w, h = f.get("innerWidth", 0), f.get("innerHeight", 0)
    if not w or not h:
        return None
    boxes = [
        (a.x - a.width / 2, a.y - a.height / 2, a.x + a.width / 2, a.y + a.height / 2)
        for a in getattr(targeter, "last_affordances", [])
    ]
    # Down the right-hand edge and along the bottom: the regions a centred
    # modal leaves clear. Ordered outside-in so the first hit is the furthest
    # from anything the modal owns.
    for fx, fy in ((0.97, 0.93), (0.97, 0.5), (0.5, 0.97), (0.03, 0.93)):
        px, py = to_screen(f, w * fx, h * fy)
        if not any(x0 <= px <= x1 and y0 <= py <= y1 for x0, y0, x1, y1 in boxes):
            return px, py
    return None


def _pointer_anchor(targeter, surface: "Surface") -> Tuple[int, int]:
    """Where to put the cursor so the wheel moves the pane in question.

    Prefer the declared control, but only while it is actually visible: an
    anchor below the fold is a cursor outside the window, and the wheel then
    reaches nothing at all. The fallback is the window's own centre, computed
    from the frame CDP reports rather than hardcoded — when every control in a
    scroll pane is below its fold there is no visible thing left to point at,
    which is exactly the state the settings modal is in.
    """
    over = surface.scroll_over
    if over is not None:
        try:
            a = targeter.find(
                over.contains, nth=over.nth,
                require_enabled=over.require_enabled, exact=over.exact,
            )
            if a.in_viewport:
                return a.x, a.y
        except LookupError:
            pass
    f = getattr(targeter, "last_frame", {}) or {}
    if not f:
        return (0, 0)
    return to_screen(f, f.get("innerWidth", 0) / 2, f.get("innerHeight", 0) / 2)


def _bring_into_view(targeter, actuator, surface: "Surface", target, settle: float):
    """Wheel the pane until the entry point is actually clickable.

    A transcription of what the driver does for a journey step, and for the
    same reasons: wheel rather than scrollIntoView so the pane moves the way it
    moves for a person; give up immediately on "covered", because the wheel
    will not move an overlay and scrolling would blame the scroll bound for
    what is really a modal in the way; and require the coordinate to hold still
    before trusting it, because the WebView animates the scroll and a position
    read mid-animation is stale by the time a click lands.
    """
    spec = surface.open_target
    if target.in_viewport or surface.scroll_over is None:
        return target
    for _ in range(getattr(spec, "scroll_attempts", 12) or 12):
        if target.in_viewport or target.off_screen == "covered":
            break
        ax, ay = _pointer_anchor(targeter, surface)
        actuator.send(move(ax, ay))
        actuator.send(scroll(240 if target.off_screen == "above" else -240))
        time.sleep(max(settle, 0.4))
        target = targeter.find(
            spec.contains, nth=spec.nth, require_enabled=spec.require_enabled, exact=spec.exact
        )
    # Hold still before it is acted on.
    for _ in range(3):
        time.sleep(max(settle, 0.4))
        again = targeter.find(
            spec.contains, nth=spec.nth, require_enabled=spec.require_enabled, exact=spec.exact
        )
        if (again.x, again.y) == (target.x, target.y):
            return again
        target = again
    return target


def _previous_surface(census: Census, current: str) -> str:
    """The surface recorded immediately before this one, or "" if none."""
    names = [n for n in census.surfaces if n != current]
    return names[-1] if names else ""


def _same_screen(previous, names_now) -> bool:
    """True when a freshly walked surface is indistinguishable from the last.

    Compared as the canonical identities the census stores, so a counter
    ticking over between two screens does not read as a difference.
    """
    from .affordance_coverage import canon_name

    before = {(a.role, a.name) for a in previous}
    after = {((r or "").strip().casefold(), canon_name(t)) for r, t in names_now}
    return bool(before) and before == after


def take_census(
    targeter,
    actuator,
    surfaces: Sequence[Surface] = tuple(SURFACES),
    *,
    language: str,
    app_version: str,
    settle: float = 1.5,
    reset_keys: Sequence[str] = ("esc",),
    on_error=None,
) -> Tuple[Census, List[str]]:
    """Walk the surfaces and record what is on each.

    Returns the census and the list of surfaces that could not be reached. An
    unreachable surface is reported rather than raised: the denominator from a
    partial walk is still worth having as long as nobody mistakes it for a
    complete one, and the caller is told exactly which screens are missing.
    """
    census = Census(language=language, app_version=app_version)
    unreached: List[str] = []
    # Leave whatever the last walk left open. Without this the census depends on
    # how the previous run ended: on 2026-08-06 a walk finished inside the
    # settings modal and the next one found every sidebar entry "covered",
    # which is true but useless.
    for key in reset_keys:
        actuator.send(press_key(key))
    if reset_keys:
        time.sleep(max(settle, 0.4))
    # Escape does not close this app's settings modal, so a walk that ended
    # inside one would otherwise poison every later run: the sidebar comes back
    # "covered" and eight surfaces go unreached. Clicking the backdrop is the
    # only handle left (the close button has no accessible name), and the point
    # is chosen to be somewhere no affordance sits.
    if reset_keys:
        try:
            targeter.affordances()
            spot = _backdrop_point(targeter)
            if spot:
                actuator.send(click(*spot))
                time.sleep(max(settle, 0.4))
        except Exception:  # noqa: BLE001 — a reset that fails must not lose the walk
            pass
    for surface in surfaces:
        try:
            if surface.open_target is not None:
                spec = surface.open_target
                target = targeter.find(
                    spec.contains,
                    nth=spec.nth,
                    require_enabled=spec.require_enabled,
                    exact=spec.exact,
                )
                target = _bring_into_view(targeter, actuator, surface, target, settle)
                # Resolving a target is not the same as being able to click it.
                # A control scrolled just past a scroll pane's edge still
                # reports a rect inside the window, and clicking it hits
                # whatever is actually on top — which for the census meant a
                # surface silently recording a byte-identical copy of the
                # previous one (observed 2026-08-06: settings-danger-zone came
                # back identical to settings-health). Counted twice, that is a
                # denominator inflated with a screen nobody ever saw.
                if not target.in_viewport:
                    raise RuntimeError(
                        f"target {target.text!r} is not clickable "
                        f"({target.off_screen or 'covered'}); the census does not "
                        "scroll, so this surface needs a reachable entry point"
                    )
                actuator.send(click(target.x, target.y))
                time.sleep(settle)
            found = targeter.affordances()
            # A surface that reproduces the previous one exactly is the same
            # screen counted twice: the navigation did not take.
            previous = census.surfaces.get(_previous_surface(census, surface.name))
            names_now = [(t.role, t.text) for t in found]
            if previous is not None and names_now and _same_screen(previous, names_now):
                raise RuntimeError(
                    "navigation did not change the screen — this surface is "
                    "identical to the one before it"
                )
            census.add_targets(surface.name, found)
        except Exception as e:  # noqa: BLE001 — one bad surface must not lose the rest
            unreached.append(f"{surface.name}: {type(e).__name__}: {str(e)[:160]}")
            if on_error:
                on_error(surface.name, e)
            continue
        finally:
            if surface.close_key:
                try:
                    actuator.send(press_key(surface.close_key))
                    time.sleep(settle)
                except Exception:  # noqa: BLE001
                    unreached.append(f"{surface.name}: failed to close")
    return census, unreached


def main(argv=None) -> int:
    from . import backends_vm as B
    from .cdp import CdpTargeter, CdpTunnel

    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--out", required=True, help="where to write the census JSON")
    p.add_argument("--language", default="ja", help="UI language the app is running in")
    p.add_argument("--settle", type=float, default=1.5)
    args = p.parse_args(argv)

    tunnel = B.SshTunnel().open()
    cdp = None
    try:
        fetch = B.TunnelJsonFetch(tunnel)
        version = str(
            (fetch("/api/system/version").get("data") or {}).get("version", "unknown")
        )
        actuator = B.TunnelActuator(tunnel)
        cdp = CdpTunnel().open()
        targeter = CdpTargeter(cdp)
        # The denominator is only meaningful if the walk actually reached the
        # surfaces it claims to have visited, and that needs the frames and the
        # coordinates to be in one pixel space (bug-503/504).
        targeter.affordances()
        problem = space_mismatch(
            targeter.last_frame, captured_size(B.TunnelScreen(tunnel))
        )
        if problem:
            print(f"census: {problem}", file=sys.stderr)
            return 3
        census, unreached = take_census(
            targeter,
            actuator,
            language=args.language,
            app_version=version,
            settle=args.settle,
        )
    finally:
        if cdp:
            cdp.close()
        tunnel.close()

    with open(args.out, "w", encoding="utf-8") as fh:
        fh.write(census.to_json() + "\n")

    total = len(census.ids())
    for surface, rows in sorted(census.surfaces.items()):
        print(f"  {surface:10s} {len(rows):3d} affordances")
    print(f"census: {total} distinct affordances over {len(census.surfaces)} surfaces")
    print(f"        language={census.language} app_version={census.app_version}")
    print(f"        written to {args.out}")
    if unreached:
        # Loud, and non-zero: a denominator quietly short by a whole screen
        # reads as better coverage than the suite has.
        print("\nUNREACHED SURFACES (the denominator is incomplete):", file=sys.stderr)
        for u in unreached:
            print(f"  - {u}", file=sys.stderr)
        return 5
    return 0


if __name__ == "__main__":
    sys.exit(main())
