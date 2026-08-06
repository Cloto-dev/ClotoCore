"""CDP targeting — resolve what the user can see to where it is on screen.

Journeys used to carry pixel coordinates (``click(455, 453)  # measured
2026-07-31``). A coordinate is only true for the scroll position, window size
and list length it was measured in, and when it drifts the click still lands
somewhere — on the backdrop, on the neighbouring checkbox — so the run fails
somewhere later, wearing a disguise. That is not hypothetical: on 2026-07-31 a
drifted click selected the widest scope instead of the intended one, and on
2026-08-05 the same class closed the modal mid-walk.

The app opens WebView2's remote-debugging port when — and only when —
``WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`` is set for the run (see
``dashboard/src-tauri/src/lib.rs``). With it open, a target is resolved at the
moment it is clicked: ask the page where the element is now, convert to screen
space, click there.

Two deliberate limits:

* **Only perceptual attributes leave this module.** A resolved target reports
  role, visible text, rect and enabled-ness — never ``data-testid`` or route
  names. An exploring agent that could read internal identifiers would know
  things a user cannot, and "the user cannot find this" is the defect class the
  apex exists to catch (Goal #211).
* **Dependency-free.** A minimal RFC6455 client rather than a package, so the
  harness keeps installing with nothing but the stdlib on the orchestrator.

The screen conversion is ``screenX + css_x * devicePixelRatio``. Verified on
2026-08-05 against a window at the origin and a window snapped to
``screenX=495``; **not** verified at a display scale other than 100 %, where
``devicePixelRatio`` is the term that would carry the correction.
"""

from __future__ import annotations

import base64
import json
import os
import socket
import struct
import subprocess
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import List, Optional

from .backends_vm import _cfg

# Port the app is told to open on the guest, and the local port the tunnel maps
# it to. Both overridable so two runs can coexist on one orchestrator.
GUEST_DEBUG_PORT = "9222"
LOCAL_DEBUG_PORT = "19222"


def debug_env() -> dict:
    """The environment a launch needs for its DOM to be reachable.

    Handed to the agent's ``POST /run`` so the *installed* artifact is the thing
    under test — no instrumented build, which would verify something other than
    what ships.
    """
    port = _cfg("OPV_CDP_GUEST_PORT", GUEST_DEBUG_PORT)
    return {"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": f"--remote-debugging-port={port}"}


class CdpTunnel:
    """An ssh ``-L`` forward from the orchestrator to the guest's debug port.

    Separate from :class:`~.backends_vm.SshTunnel` (agent + kernel ports): the
    debug port only exists for runs that asked for it, and a journey that does
    not need the DOM should not fail because it is closed.
    """

    def __init__(self):
        self.local = int(_cfg("OPV_CDP_LOCAL_PORT", LOCAL_DEBUG_PORT))
        self.guest = _cfg("OPV_CDP_GUEST_PORT", GUEST_DEBUG_PORT)
        self.vm = f"{_cfg('OPV_VM_USER', 'PC')}@{_cfg('OPV_VM_IP', '192.168.0.252')}"
        self._ctl = f"/tmp/opv-cdp-{self.local}-%r@%h:%p"

    def open(self, timeout: float = 20.0) -> "CdpTunnel":
        subprocess.run(
            [
                "ssh", "-f", "-N", "-M",
                "-o", "ExitOnForwardFailure=yes",
                "-o", "StrictHostKeyChecking=accept-new",
                "-o", f"ControlPath={self._ctl}",
                "-L", f"{self.local}:127.0.0.1:{self.guest}",
                self.vm,
            ],
            check=True,
            capture_output=True,
            timeout=timeout,
        )
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                self.version()
                return self
            except Exception:  # noqa: BLE001 - the forward may not be up yet
                time.sleep(0.5)
        raise RuntimeError(
            f"the debug port did not answer through the forward within {timeout:.0f}s "
            "— was the app launched with debug_env()?"
        )

    def close(self) -> None:
        subprocess.run(
            ["ssh", "-O", "exit", "-o", f"ControlPath={self._ctl}", self.vm],
            capture_output=True,
        )

    def __enter__(self) -> "CdpTunnel":
        return self.open()

    def __exit__(self, *exc) -> None:
        self.close()

    # -- HTTP side of the protocol -------------------------------------
    def _get(self, path: str, timeout: float = 10.0):
        url = f"http://127.0.0.1:{self.local}{path}"
        with urllib.request.urlopen(url, timeout=timeout) as r:
            return json.loads(r.read().decode())

    def version(self):
        return self._get("/json/version")

    def page_target(self) -> dict:
        pages = [t for t in self._get("/json") if t.get("type") == "page"]
        if not pages:
            raise RuntimeError("no page target on the debug port")
        return pages[0]


class _WebSocket:
    """The smallest client that can carry CDP: text frames, client-masked, no
    fragmentation. CDP messages are small JSON documents, so nothing here needs
    continuation frames or compression."""

    def __init__(self, url: str, local_port: int, timeout: float = 15.0):
        path = url.split("://", 1)[1].partition("/")[2]
        self.sock = socket.create_connection(("127.0.0.1", local_port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(
            (
                f"GET /{path} HTTP/1.1\r\n"
                f"Host: 127.0.0.1:{local_port}\r\n"
                "Upgrade: websocket\r\nConnection: Upgrade\r\n"
                f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            ).encode()
        )
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise RuntimeError("debug socket closed during handshake")
            buf += chunk
        status = buf.split(b"\r\n", 1)[0]
        if b"101" not in status:
            raise RuntimeError(f"websocket handshake refused: {status!r}")
        self._rest = buf.split(b"\r\n\r\n", 1)[1]
        self._id = 0

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass

    def _recv_exact(self, n: int) -> bytes:
        while len(self._rest) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RuntimeError("debug socket closed")
            self._rest += chunk
        out, self._rest = self._rest[:n], self._rest[n:]
        return out

    def call(self, method: str, params: Optional[dict] = None) -> dict:
        self._id += 1
        want = self._id
        payload = json.dumps({"id": want, "method": method, "params": params or {}}).encode()
        mask = os.urandom(4)
        n = len(payload)
        if n < 126:
            head = struct.pack("!BB", 0x81, 0x80 | n)
        elif n < (1 << 16):
            head = struct.pack("!BBH", 0x81, 0x80 | 126, n)
        else:
            head = struct.pack("!BBQ", 0x81, 0x80 | 127, n)
        self.sock.sendall(
            head + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        )
        while True:
            b0, b1 = struct.unpack("!BB", self._recv_exact(2))
            size = b1 & 0x7F
            if size == 126:
                size = struct.unpack("!H", self._recv_exact(2))[0]
            elif size == 127:
                size = struct.unpack("!Q", self._recv_exact(8))[0]
            data = self._recv_exact(size)
            if (b0 & 0x0F) != 1:  # ignore ping/pong/close-adjacent frames
                continue
            msg = json.loads(data.decode())
            # CDP interleaves events with replies; only the matching id is ours.
            if msg.get("id") != want:
                continue
            if "error" in msg:
                raise RuntimeError(f"{method}: {msg['error']}")
            return msg.get("result", {})


@dataclass
class Target:
    """Where a perceivable element is, right now."""

    text: str
    role: str
    x: int  # screen coordinates, ready for the actuator
    y: int
    width: int
    height: int
    enabled: bool
    # True only when a click at (x, y) would actually reach this element: fully
    # inside every clipping ancestor *and* the top thing at that point. The name
    # is historical; "the window contains its rect" was the weaker test that let
    # three runs click a modal's backdrop.
    in_viewport: bool
    # "" when actionable; "above"/"below" for which way the wheel must go;
    # "covered" when something is on top of it, which scrolling will not fix.
    off_screen: str = ""


# Only perceptual attributes cross this boundary — see the module docstring.
_AFFORDANCES = r"""
(() => {
  const SEL = 'button, a[href], input, select, textarea, [role=button], [role=tab], [role=link]';

  // The window is not what an element is visible *within*. A control scrolled
  // just past the bottom of a modal's scroll pane still has a rect inside the
  // window, and `getBoundingClientRect` reports it happily — it is the ancestor
  // with `overflow` that is hiding it. Measured 2026-08-05: the uninstall button
  // resolved to y=662 while the card ended at y≈628, so the click went to the
  // backdrop and closed the modal, three runs in a row, each of which read as
  // "the app refused to uninstall".
  const clipOf = (el) => {
    let c = {top: 0, left: 0, bottom: innerHeight, right: innerWidth};
    for (let p = el.parentElement; p; p = p.parentElement) {
      const cs = getComputedStyle(p);
      if (/auto|scroll|hidden/.test(cs.overflowY + ' ' + cs.overflowX)) {
        const pr = p.getBoundingClientRect();
        c = {top: Math.max(c.top, pr.top), left: Math.max(c.left, pr.left),
             bottom: Math.min(c.bottom, pr.bottom), right: Math.min(c.right, pr.right)};
      }
    }
    return c;
  };

  const out = [];
  for (const el of document.querySelectorAll(SEL)) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    const cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none' || cs.opacity === '0') continue;
    // A checkbox has no text of its own; what names it, for a person, is the
    // label beside it. Without this a scope checkbox is unaddressable while
    // being the most obvious control on the screen (measured 2026-08-05: the
    // tier widening could not be resolved and the run executed at tier 1).
    const labelled = (el.labels && el.labels[0] ? el.labels[0].innerText : '')
      || (el.closest('label') ? el.closest('label').innerText : '');
    const cx = Math.round(r.x + r.width / 2), cy = Math.round(r.y + r.height / 2);
    const c = clipOf(el);
    const within = r.top >= c.top && r.bottom <= c.bottom && r.left >= c.left && r.right <= c.right;
    // The definitive test: what would a click at that point actually hit? This
    // is also what catches an overlay covering a control that is otherwise
    // perfectly in view.
    const hit = document.elementFromPoint(cx, cy);
    const hittable = !!hit && (hit === el || el.contains(hit) || hit.contains(el));
    out.push({
      role: el.getAttribute('role') || el.tagName.toLowerCase(),
      text: (el.innerText || el.getAttribute('aria-label') || el.placeholder || labelled || '')
        .trim().slice(0, 80),
      cx: cx, cy: cy,
      w: Math.round(r.width), h: Math.round(r.height),
      enabled: !el.disabled,
      inViewport: within && hittable,
      offScreen: r.bottom > c.bottom ? 'below' : (r.top < c.top ? 'above' : (hittable ? '' : 'covered'))
    });
  }
  return JSON.stringify({
    frame: {screenX: window.screenX, screenY: window.screenY, dpr: devicePixelRatio,
            innerWidth: innerWidth, innerHeight: innerHeight},
    affordances: out
  });
})()
"""


class CdpTargeter:
    """Resolves visible text to a screen coordinate, per call, live.

    A fresh websocket per resolution: the page target survives navigation inside
    the app, but a socket held across a purge — which ends the process — would
    fail in a way that looks like a resolution failure rather than the expected
    end of the app.
    """

    def __init__(self, tunnel: CdpTunnel):
        self.tunnel = tunnel
        self.last_affordances: List[Target] = []
        self.last_frame: dict = {}

    def affordances(self) -> List[Target]:
        target = self.tunnel.page_target()
        ws = _WebSocket(target["webSocketDebuggerUrl"], self.tunnel.local)
        try:
            res = ws.call(
                "Runtime.evaluate", {"expression": _AFFORDANCES, "returnByValue": True}
            )
        finally:
            ws.close()
        data = json.loads(res["result"]["value"])
        f = data["frame"]
        # Kept so a caller can aim the pointer at the window itself. Every
        # affordance is a *thing*, and when the only controls inside a scroll
        # pane are below its fold there is no visible thing to point at — the
        # census hit exactly that in the settings modal (2026-08-06) and moved
        # the cursor to y=915 on an 800px screen, so the wheel reached nothing.
        self.last_frame = f
        self.last_affordances = [
            Target(
                text=a["text"],
                role=a["role"],
                x=round(f["screenX"] + a["cx"] * f["dpr"]),
                y=round(f["screenY"] + a["cy"] * f["dpr"]),
                width=a["w"],
                height=a["h"],
                enabled=a["enabled"],
                in_viewport=a["inViewport"],
                off_screen=a.get("offScreen", ""),
            )
            for a in data["affordances"]
        ]
        return self.last_affordances

    def find(
        self,
        contains,
        *,
        nth: int = 0,
        require_enabled: bool = False,
        exact: bool = False,
    ) -> Target:
        """The `nth` visible element whose text contains `contains`.

        `contains` may be a tuple of alternatives, matched case-insensitively —
        the VM runs the Japanese pack while the locale files are authored in
        English, and buttons are rendered through `text-transform: uppercase`, so
        a journey pinned to one exact string is pinned to one locale and one
        stylesheet.

        Raises with the list of what *was* on screen. A resolution failure has to
        be loud: silently falling back to a coordinate is how a click ends up on
        the backdrop.
        """
        wanted = (contains,) if isinstance(contains, str) else tuple(contains)
        lowered = [w.lower() for w in wanted]
        if exact:
            # `exact` exists to separate a control from one that merely contains
            # its words. The confirm dialog's button is "Uninstall" while the
            # card behind it says "Uninstall ClotoCore"; a substring match picks
            # whichever comes first in the DOM — on 2026-08-05 that was the one
            # behind the modal, so the confirmation was never given and the run
            # blamed the app for not exiting.
            def matches(t: Target) -> bool:
                return t.text.strip().lower() in lowered
        else:
            def matches(t: Target) -> bool:
                return any(w in t.text.lower() for w in lowered)

        hits = [t for t in self.affordances() if matches(t)]
        if require_enabled:
            hits = [t for t in hits if t.enabled]
        if len(hits) <= nth:
            seen = ", ".join(repr(t.text) for t in self.last_affordances if t.text)
            raise LookupError(
                f"no target[{nth}] whose text contains any of {wanted!r}"
                f"{' (enabled)' if require_enabled else ''}; on screen: {seen[:600]}"
            )
        return hits[nth]
