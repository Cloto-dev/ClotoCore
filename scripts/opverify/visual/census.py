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
from .interfaces import click, press_key
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
    """

    name: str
    open_target: Optional[TargetSpec] = None
    close_key: Optional[str] = None

    def __post_init__(self):
        if self.open_target is None:
            return
        contains = self.open_target.contains
        alts = (contains,) if isinstance(contains, str) else tuple(contains or ())
        if not is_safe_nav(alts):
            raise UnsafeNavigation(
                f"surface {self.name!r} navigates by clicking {alts!r}, which names "
                "an action. The census may only click its way between screens."
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
    # Last, and it closes itself: everything after a modal would otherwise be
    # censused through an overlay.
    Surface("settings", TargetSpec(contains=("設定", "settings")), close_key="esc"),
]


def take_census(
    targeter,
    actuator,
    surfaces: Sequence[Surface] = tuple(SURFACES),
    *,
    language: str,
    app_version: str,
    settle: float = 1.5,
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
                actuator.send(click(target.x, target.y))
                time.sleep(settle)
            census.add_targets(surface.name, targeter.affordances())
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
