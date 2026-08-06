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


# Captured before any scenario swaps it out: the status-handling tests need the
# real implementation, and by the time they run the earlier scenarios have
# replaced the module attribute with a stub.
_REAL_HTTP_GET = B._http_get


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


class _FakeResponse:
    def __init__(self, status: int, body: bytes):
        self.status = status
        self._body = body

    def read(self) -> bytes:
        return self._body


class _FakeConn:
    """Stands in for http.client.HTTPConnection so the status handling in
    `_http_get` can be exercised without a socket."""

    status = 200
    body = b'{"data":{"providers":[]}}'

    def __init__(self, host, port, timeout=None):
        pass

    def request(self, method, path, headers=None, body=None):
        pass

    def getresponse(self):
        return _FakeResponse(_FakeConn.status, _FakeConn.body)

    def close(self):
        pass


def scenario_non_2xx_is_not_a_body() -> None:
    """A 403 must not be handed back as data (bug-500).

    Returning the body let `.get("providers", [])` read a rejected request as
    a machine with no providers — an absent state rather than an absent
    answer. The status is only visible here, so this is where it is enforced.
    """
    import http.client

    from .interfaces import ProbeUnavailable

    orig = http.client.HTTPConnection
    http.client.HTTPConnection = _FakeConn
    try:
        _FakeConn.status, _FakeConn.body = 200, b'{"data":{"ok":true}}'
        assert _REAL_HTTP_GET(18081, "/api/llm/providers") == b'{"data":{"ok":true}}'

        _FakeConn.status, _FakeConn.body = 403, b'{"error":"forbidden"}'
        try:
            _REAL_HTTP_GET(18081, "/api/llm/providers")
        except ProbeUnavailable as e:
            assert e.status == 403, e.status
            # The message has to point at the credential, because that is the
            # cause an operator can act on.
            assert "OPV_API_KEY" in str(e), str(e)
        else:
            raise AssertionError("a 403 body was returned as if it were state")
    finally:
        http.client.HTTPConnection = orig
        _FakeConn.status, _FakeConn.body = 200, b'{"data":{"providers":[]}}'


def scenario_unset_key_refuses_to_probe() -> None:
    """`OPV_API_KEY` defaults to empty, so forgetting it used to send a keyless
    request and collect a 403 — the silent first step of the same failure. The
    authenticated probes must refuse before the request goes out, and the
    unauthenticated ones must be unaffected."""
    from .interfaces import ProbeUnavailable

    for name, call in (
        ("TunnelJsonFetch", lambda: B.TunnelJsonFetch(_StubTunnel(), "")("/api/llm/providers")),
        ("TunnelApiProbe", lambda: B.TunnelApiProbe(_StubTunnel(), "/api/history", "x", "").check()),
        ("KernelApiProbe", lambda: B.KernelApiProbe("/api/history", "x", "").check()),
        ("KernelJsonFetch", lambda: B.KernelJsonFetch("")("/api/llm/providers")),
    ):
        try:
            call()
        except ProbeUnavailable as e:
            assert "OPV_API_KEY" in str(e), (name, str(e))
        else:
            raise AssertionError(f"{name} probed with no credential")

    # Liveness is unauthenticated: it must still work with no key at all.
    _install_http(get=lambda p, path, h: b'{"data":{"status": "ok"}}')
    assert B.TunnelHealthProbe(_StubTunnel()).check() is True


def scenario_status_trailer_parsing() -> None:
    """The curl transport carries the status in a trailer, so a body that
    happens to end in a newline, or a reply with no trailer at all, must not
    be read as a success."""
    assert B._split_status(b'{"a":1}\n200') == (b'{"a":1}', 200)
    assert B._split_status(b'{"a":1}\n\n403') == (b'{"a":1}\n', 403)
    # No trailer → unknown, not 200. Callers treat 0 as "could not ask".
    assert B._split_status(b'{"a":1}')[1] == 0
    assert B._split_status(b'{"a":1}\nnot-a-status')[1] == 0


def main() -> int:
    orig_get, orig_post = B._http_get, B._http_post
    scenarios = [
        scenario_grab,
        scenario_actuator,
        scenario_health,
        scenario_api_probe,
        scenario_hash,
        scenario_non_2xx_is_not_a_body,
        scenario_unset_key_refuses_to_probe,
        scenario_status_trailer_parsing,
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
