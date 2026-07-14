"""Self-test of the SSH port-forward transport backends (#235) without a real
VM/ssh — the HTTP helpers are monkeypatched. Run:
``python -m scripts.opverify.visual.tunnel_selftest`` (exit 0 = all passed).

Covers: grab returns the raw PNG frame; actuator posts the right /act JSON and
rejects a non-ok reply; the health probe parses (space-insensitively) and is a
False gate when the socket errors; the api probe carries X-API-Key; the hash
source parses /grabhash. SshTunnel's ssh lifecycle needs a real host and is
exercised by the live run_vm, not here — the backends only read the tunnel's
local_agent/local_kernel ports, so a tiny stub stands in.
"""

from __future__ import annotations

import sys

from . import backends_vm as B
from .interfaces import Action


class _StubTunnel:
    local_agent = 18900
    local_kernel = 18081


def _install_http(get=None, post=None):
    calls = {"get": [], "post": []}

    def _get(port, path, headers=None, timeout=15.0):
        calls["get"].append((port, path, headers))
        return get(port, path, headers) if get else b""

    def _post(port, path, body, timeout=15.0):
        calls["post"].append((port, path, body))
        return post(port, path, body) if post else b'{"ok": true}'

    B._http_get = _get
    B._http_post = _post
    return calls


def scenario_grab() -> None:
    png = b"\x89PNG\r\n\x1a\nDATA"
    _install_http(get=lambda p, path, h: png)
    frame = B.TunnelScreen(_StubTunnel()).grab()
    assert frame.data == png, frame.data
    assert frame.fingerprint  # sha256 computed


def scenario_actuator() -> None:
    calls = _install_http(post=lambda p, path, body: b'{"ok": true, "kind": "click"}')
    B.TunnelActuator(_StubTunnel()).send(Action("click", x=5, y=9))
    port, path, body = calls["post"][0]
    assert port == 18900 and path == "/act", (port, path)
    assert b'"kind": "click"' in body and b'"x": 5' in body and b'"y": 9' in body, body

    # a non-ok reply must raise
    _install_http(post=lambda p, path, body: b'{"ok": false, "error": "nope"}')
    try:
        B.TunnelActuator(_StubTunnel()).send(Action("click", x=1, y=2))
        raise AssertionError("expected /act rejection to raise")
    except RuntimeError:
        pass


def scenario_health() -> None:
    calls = _install_http(get=lambda p, path, h: b'{"data":{"status": "ok"}}')
    ok = B.TunnelHealthProbe(_StubTunnel()).check()
    assert ok is True
    port, path, _ = calls["get"][0]
    assert port == 18081 and path == "/api/system/health", (port, path)

    # socket error → False gate
    def boom(port, path, headers=None, timeout=15.0):
        raise OSError("connection refused")

    B._http_get = boom
    assert B.TunnelHealthProbe(_StubTunnel()).check() is False


def scenario_api_probe() -> None:
    calls = _install_http(get=lambda p, path, h: b'[{"agent_type":"agent"}]')
    ok = B.TunnelApiProbe(
        _StubTunnel(), "/api/agents", '"agent_type":"agent"', "KEY123"
    ).check()
    assert ok is True
    port, path, headers = calls["get"][0]
    assert port == 18081 and headers == {"X-API-Key": "KEY123"}, (port, headers)


def scenario_hash() -> None:
    _install_http(get=lambda p, path, h: b'{"ok": true, "hash": "deadbeef"}')
    assert B.TunnelHashSource(_StubTunnel()).hash() == "deadbeef"


def main() -> int:
    orig_get, orig_post = B._http_get, B._http_post
    scenarios = [
        scenario_grab,
        scenario_actuator,
        scenario_health,
        scenario_api_probe,
        scenario_hash,
    ]
    try:
        for sc in scenarios:
            sc()
            print(f"  ok  {sc.__name__}")
    finally:
        B._http_get, B._http_post = orig_get, orig_post
    print(f"tunnel selftest: {len(scenarios)}/{len(scenarios)} scenarios passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
