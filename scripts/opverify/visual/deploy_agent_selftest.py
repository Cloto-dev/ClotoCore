"""Self-test of the reproducible agent deploy (#238) without a real VM — the
ssh/scp round trips are monkeypatched. Run:
``python -m scripts.opverify.visual.deploy_agent_selftest`` (exit 0 = all passed).

Covers: the protocol version is parsed from the committed vm_agent.py source; a
full deploy issues the expected VM steps in order (dir -> deps -> scp -> register
-> restart -> health) and the health check asserts the running version equals the
committed one; a redeploy skips dir/deps/register (copy + restart + verify only);
the registered task carries the Interactive / RunLevel-Highest / AtLogOn recipe
and the resolved pythonw path and leaks no secret; status() is a MATCH/DRIFT
gate; and a version mismatch at /health fails the deploy loudly. The ssh/scp
lifecycle against a live host is exercised by the real deploy, not here.
"""

from __future__ import annotations

import sys

from . import deploy_agent as D

_PYW = r"C:\Program Files\Python312\pythonw.exe"
_PY = r"C:\Program Files\Python312\python.exe"


def _install(health_version=None):
    """Fake _ssh/_scp that dispatch by command content and record every call.
    health_version defaults to the committed expected version (so the deploy's
    version assertion passes); override to force DRIFT."""
    calls = {"ssh": [], "scp": []}
    want = D._expected_version() if health_version is None else health_version

    def fake_ssh(ps, timeout=60.0):
        calls["ssh"].append(ps)
        if "Get-Command python" in ps:
            return f"{_PY}|{_PYW}"
        if "/health" in ps:
            return (
                '{"ok": true, "version": %d, "session": 1, "screen": [1280, 800]}'
                % want
            )
        return "ok"

    def fake_scp(local, remote_fwd, timeout=60.0):
        calls["scp"].append((local, remote_fwd))

    D._ssh = fake_ssh
    D._scp = fake_scp
    D.time.sleep = lambda *_: None
    return calls


def scenario_version() -> None:
    v = D._expected_version()
    assert isinstance(v, int) and v >= 1, v
    # matches vm_agent.py's /health handler (currently 3)
    assert v == 3, f"expected committed agent version 3, got {v}"


def scenario_full_deploy() -> None:
    calls = _install()
    rc = D.deploy(redeploy=False)
    assert rc == 0, rc
    joined = "\n".join(calls["ssh"])
    # steps present
    assert "New-Item -ItemType Directory" in joined
    assert "pip install" in joined and "mss pyautogui" in joined
    assert "Register-ScheduledTask" in joined
    assert "Start-ScheduledTask" in joined
    assert "/health" in joined
    # scp targets the canonical remote path
    assert calls["scp"] and calls["scp"][0][1] == D.REMOTE_AGENT_SCP, calls["scp"]
    assert calls["scp"][0][0].endswith("vm_agent.py"), calls["scp"]
    # ordering: dir created before the scp copy; register before restart
    dir_i = next(i for i, c in enumerate(calls["ssh"]) if "New-Item" in c)
    reg_i = next(i for i, c in enumerate(calls["ssh"]) if "Register-ScheduledTask" in c)
    start_i = next(i for i, c in enumerate(calls["ssh"]) if "Start-ScheduledTask" in c)
    assert dir_i < reg_i < start_i, (dir_i, reg_i, start_i)


def scenario_register_recipe_and_no_secret() -> None:
    calls = _install()
    D.deploy(redeploy=False)
    reg = next(c for c in calls["ssh"] if "Register-ScheduledTask" in c)
    assert "-LogonType Interactive" in reg, reg
    assert "-RunLevel Highest" in reg, reg
    assert "New-ScheduledTaskTrigger -AtLogOn" in reg, reg
    assert D.TASK_NAME in reg
    assert _PYW in reg, reg  # resolved pythonw path, not a bare 'python'
    assert D.REMOTE_AGENT in reg
    # secret hygiene: the deploy is env-driven and must not carry credentials
    low = "\n".join(calls["ssh"]).lower()
    assert "api_key" not in low and "password" not in low and "seal" not in low


def scenario_redeploy_skips_setup() -> None:
    calls = _install()
    rc = D.deploy(redeploy=True)
    assert rc == 0, rc
    joined = "\n".join(calls["ssh"])
    assert "Register-ScheduledTask" not in joined, "redeploy must not re-register"
    assert "pip install" not in joined, "redeploy must not reinstall deps"
    assert calls["scp"], "redeploy must still copy the agent"
    assert "Start-ScheduledTask" in joined and "/health" in joined


def scenario_status_gate() -> None:
    _install()  # health matches expected
    assert D.status() == 0
    _install(health_version=D._expected_version() + 1)  # drift
    assert D.status() == 1


def scenario_version_mismatch_fails() -> None:
    _install(health_version=D._expected_version() + 1)
    try:
        D.deploy(redeploy=False)
        raise AssertionError("expected deploy to fail on version drift")
    except RuntimeError:
        pass


def main() -> int:
    orig_ssh, orig_scp = D._ssh, D._scp
    scenarios = [
        scenario_version,
        scenario_full_deploy,
        scenario_register_recipe_and_no_secret,
        scenario_redeploy_skips_setup,
        scenario_status_gate,
        scenario_version_mismatch_fails,
    ]
    try:
        for sc in scenarios:
            sc()
            print(f"  ok  {sc.__name__}")
    finally:
        D._ssh, D._scp = orig_ssh, orig_scp
    print(f"deploy_agent selftest: {len(scenarios)}/{len(scenarios)} scenarios passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
