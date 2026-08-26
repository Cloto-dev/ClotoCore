"""Self-test of the CSS -> screen conversion (CSC Task #530), no VM needed.
Run: ``python -m scripts.opverify.visual.cdp_selftest`` (exit 0 = all passed).

The bug this guards was live for four days and never failed a run, because
every run happened on a display at 100 % scale, where the wrong formula and the
right one are the same arithmetic. A test that inherits the machine's scale
would have been just as blind, so each scenario below *states* a scale.

The numbers are not invented: they are the frames measured on the Windows verify
guest on 2026-08-09, and the assertions are the pointer positions the page
reported for those aims.
"""

from __future__ import annotations

import sys

from .cdp import space_mismatch, to_screen


def scenario_at_100_percent_the_conversion_is_a_translation() -> None:
    """dpr 1: screen = window origin + CSS offset, and nothing else.

    This is the case every apex run to date has exercised, so it is also the
    case a regression would keep passing. It is here to pin the floor, not
    because it was ever in doubt.
    """
    frame = {"screenX": 0, "screenY": 0, "dpr": 1}
    assert to_screen(frame, 320, 188) == (320, 188)

    offset = {"screenX": 495, "screenY": 60, "dpr": 1}
    assert to_screen(offset, 100, 40) == (595, 100)


def scenario_at_125_percent_the_origin_scales_too() -> None:
    """dpr 1.25 with the window off the origin -- the case that separates the
    candidate formulas.

    Measured frame: the app at screenX=307, screenY=81 on the Windows guest.
    Aiming at the CSS point (156, 118) has to put the physical pointer at
    (579, 249); the page then reported clientX/clientY of exactly (156, 118).

    The formula this replaced (``screenX + cx*dpr``) yields (502, 228) for the
    same input, which the page reported as CSS (195, 147) -- 39 CSS pixels off,
    enough to land on a neighbouring control.
    """
    frame = {"screenX": 307, "screenY": 81, "dpr": 1.25}
    assert to_screen(frame, 156, 118) == (579, 249)
    assert to_screen(frame, 313, 236) == (775, 396)
    assert to_screen(frame, 470, 354) == (971, 544)

    # The superseded formula, spelled out so the difference is visible here
    # rather than only in a commit message.
    wrong = (
        round(frame["screenX"] + 156 * frame["dpr"]),
        round(frame["screenY"] + 118 * frame["dpr"]),
    )
    assert wrong == (502, 228)
    assert wrong != to_screen(frame, 156, 118)


def scenario_a_scale_the_vm_cannot_reach_is_still_covered() -> None:
    """dpr 1.5.

    The verify guest's 1280x800 display does not offer 150 % -- Windows caps the
    list so the logical resolution stays at least 800x600, and 150 % would make
    it 853x533 (measured 2026-08-09: index 2, 3 and 4 all clamp to 125 %). The
    arithmetic still has to be right there, and a machine that cannot be put
    into that state is exactly why this assertion is a unit test.
    """
    frame = {"screenX": 200, "screenY": 100, "dpr": 1.5}
    assert to_screen(frame, 400, 200) == (900, 450)
    assert to_screen(frame, 0, 0) == (300, 150)


def scenario_the_window_origin_is_never_dropped() -> None:
    """A conversion that ignored the origin would pass every centred-window
    test: at screenX=0 the origin term contributes nothing. Assert it moves."""
    a = to_screen({"screenX": 0, "screenY": 0, "dpr": 1.25}, 100, 100)
    b = to_screen({"screenX": 80, "screenY": 40, "dpr": 1.25}, 100, 100)
    assert b != a
    assert b == (225, 175)


def scenario_a_virtualised_agent_is_caught_before_the_first_aim() -> None:
    """The conversion is only right while the agent works in physical pixels,
    and the agent is deployed separately -- a VM left on an older copy silently
    moves back to the virtualised space. The frames say so: a 1280x800 panel
    captures as 1024x640 at 125 %.
    """
    frame = {"screenWidth": 1024, "screenHeight": 640, "dpr": 1.25}
    problem = space_mismatch(frame, (1024, 640))
    assert problem, "a virtualised capture at 125 % has to be reported"
    assert "deploy_agent --redeploy" in problem, problem

    # The aware agent at the same scale: page reports the logical screen, the
    # capture is physical, and they reconcile through the scale.
    assert space_mismatch(frame, (1280, 800)) == ""


def scenario_the_guard_stays_quiet_at_100_percent() -> None:
    """At 100 % the two spaces genuinely are one, for an aware agent and an
    unaware one alike. A guard that fired here would be telling every existing
    run to redeploy for no reason -- and would be turned off."""
    frame = {"screenWidth": 1280, "screenHeight": 800, "dpr": 1}
    assert space_mismatch(frame, (1280, 800)) == ""


def scenario_an_unanswerable_check_is_not_a_failure() -> None:
    """No frame, or a grab that could not be taken, means the check never ran.
    Reporting that as a mismatch would send an operator to redeploy a healthy
    agent over a dead transport (bug-500's lesson, same shape)."""
    assert space_mismatch({}, (1280, 800)) == ""
    assert space_mismatch({"screenWidth": 1280, "dpr": 1}, (None, None)) == ""
    assert space_mismatch({"screenWidth": 1280, "dpr": 1}, ()) == ""


def scenario_a_frame_reports_its_own_size() -> None:
    """The guard reads the capture's dimensions off the PNG, so a frame that
    is not a PNG has to stay silent rather than raise."""
    from .interfaces import Frame

    png = (
        b"\x89PNG\r\n\x1a\n"
        + b"\x00\x00\x00\x0dIHDR"
        + (1280).to_bytes(4, "big")
        + (800).to_bytes(4, "big")
    )
    f = Frame.of(png)
    assert (f.width, f.height) == (1280, 800), (f.width, f.height)

    stub = Frame.of(b"not-a-png")
    assert (stub.width, stub.height) == (None, None)


def main() -> int:
    scenarios = [
        scenario_at_100_percent_the_conversion_is_a_translation,
        scenario_at_125_percent_the_origin_scales_too,
        scenario_a_scale_the_vm_cannot_reach_is_still_covered,
        scenario_the_window_origin_is_never_dropped,
        scenario_a_virtualised_agent_is_caught_before_the_first_aim,
        scenario_the_guard_stays_quiet_at_100_percent,
        scenario_an_unanswerable_check_is_not_a_failure,
        scenario_a_frame_reports_its_own_size,
    ]
    for sc in scenarios:
        sc()
        print(f"  ok  {sc.__name__}")
    print(f"cdp selftest: {len(scenarios)}/{len(scenarios)} scenarios passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
