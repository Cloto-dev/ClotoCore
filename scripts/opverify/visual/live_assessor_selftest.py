"""Self-test of the live agent-handshake assessor (#237) without a real agent —
the responder is simulated by writing the verdict file mid-poll. Run:
``python -m scripts.opverify.visual.live_assessor_selftest`` (exit 0 = passed).
"""

from __future__ import annotations

import json
import os
import sys
import tempfile

from .interfaces import Frame
from .live_assessor import AgentHandshakeAssessor


def _clock():
    t = {"v": 0.0}
    return t


def scenario_roundtrip() -> None:
    d = tempfile.mkdtemp(prefix="opv-exch.")
    t = _clock()
    calls = {"n": 0}

    def sleep(dt):
        t["v"] += dt
        calls["n"] += 1
        if calls["n"] == 2:  # the agent writes its verdict after two polls
            with open(os.path.join(d, "resp_000.json"), "w") as f:
                json.dump({"visible": True, "detail": "GUI rendered", "defects": []}, f)

    a = AgentHandshakeAssessor(
        d, poll=0.5, timeout=30.0, now=lambda: t["v"], sleep=sleep
    )
    res = a.assess(Frame.of(b"\x89PNGdata"), "is the GUI rendered?")
    assert res.visible is True and res.detail == "GUI rendered", res

    req = json.load(open(os.path.join(d, "req_000.json")))
    assert req["seq"] == 0 and req["question"] == "is the GUI rendered?", req
    assert os.path.exists(req["frame"]), "frame PNG must be written for the agent"
    assert open(req["frame"], "rb").read() == b"\x89PNGdata"


def scenario_defects_and_seq() -> None:
    d = tempfile.mkdtemp(prefix="opv-exch.")
    t = _clock()

    # pre-place both verdicts; assess consumes seq 0 then seq 1
    with open(os.path.join(d, "resp_000.json"), "w") as f:
        json.dump({"visible": False, "detail": "spinner", "defects": ["stuck"]}, f)
    with open(os.path.join(d, "resp_001.json"), "w") as f:
        json.dump({"visible": True}, f)  # detail/defects optional

    a = AgentHandshakeAssessor(d, now=lambda: t["v"], sleep=lambda dt: None)
    r0 = a.assess(Frame.of(b"a"), "q0")
    r1 = a.assess(Frame.of(b"b"), "q1")
    assert r0.visible is False and r0.defects == ["stuck"], r0
    assert r1.visible is True and r1.detail == "" and r1.defects == [], r1


def scenario_timeout() -> None:
    d = tempfile.mkdtemp(prefix="opv-exch.")
    t = _clock()
    a = AgentHandshakeAssessor(
        d,
        poll=0.5,
        timeout=2.0,
        now=lambda: t["v"],
        sleep=lambda dt: t.__setitem__("v", t["v"] + dt),
    )
    try:
        a.assess(Frame.of(b"x"), "q")
        raise AssertionError("expected TimeoutError when no verdict is written")
    except TimeoutError:
        pass


def main() -> int:
    scenarios = [scenario_roundtrip, scenario_defects_and_seq, scenario_timeout]
    for sc in scenarios:
        sc()
        print(f"  ok  {sc.__name__}")
    print(f"live-assessor selftest: {len(scenarios)}/{len(scenarios)} scenarios passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
