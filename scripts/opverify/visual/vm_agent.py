"""opv_agent — session-1-resident actuator/perception agent for the opverify
visual apex. Runs inside the interactive Windows desktop (launched via a
Task Scheduler Interactive principal) so mss can screenshot the real screen
and pyautogui can inject OS-level input.

It listens on 127.0.0.1:8900. The Mac-side driver reaches it over SSH by
cur/l-ing VM localhost (the network socket bridges the session-0 SSH shell to
this session-1 process — the session wall only blocks GUI/input APIs, not
sockets).

Endpoints:
  GET  /health            -> {"ok":true,"session":N,"screen":[w,h]}
  GET  /grab              -> image/png of the primary monitor
  GET  /grabhash          -> {"ok":true,"hash":"<sha256 of raw pixels>"}
  POST /act   {kind,...}  -> inject input; kind in click/move/type/key/hotkey/scroll
  POST /run   {path,args} -> launch a program in this (interactive) session
  POST /quit              -> stop the agent
"""

import ctypes
import hashlib
import json
import os
import subprocess
import sys
import traceback
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Declare per-monitor DPI awareness before anything measures the screen.
# Windows hands a DPI-unaware process virtualised metrics: at 125 % scaling the
# 1280x800 panel is reported as 1024x640, /grab comes back downscaled, and every
# injected coordinate is read in that shrunken space -- so a frame, an ROI or a
# coordinate measured at 100 % silently means something else at 125 %. Awareness
# keeps this agent in physical pixels at any scale (measured 2026-08-09, CSC
# Best effort: an OS without the call runs as before.
try:
    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    pass

import mss  # noqa: E402
import mss.tools  # noqa: E402
import pyautogui  # noqa: E402

pyautogui.FAILSAFE = False
pyautogui.PAUSE = 0.05

LOG = r"C:\opv\agent.log"
PORT = 8900


def log(msg):
    try:
        with open(LOG, "a", encoding="utf-8") as f:
            f.write(str(msg) + "\n")
    except Exception:
        pass


def session_id():
    try:
        return ctypes.windll.kernel32.WTSGetActiveConsoleSessionId()
    except Exception:
        return -1


def grab_png():
    with mss.mss() as sct:
        mon = sct.monitors[1]  # primary monitor (index 0 = virtual "all")
        shot = sct.grab(mon)
        return mss.tools.to_png(shot.rgb, shot.size)


def grab_hash():
    """A cheap change-signal for settle polling: hash the raw framebuffer
    pixels directly, skipping the PNG encode and the ~85 KB body transfer that
    /grab pays. settle only needs a stable/changed signal, so it polls this and
    grabs a real PNG once the screen has stopped moving."""
    with mss.mss() as sct:
        shot = sct.grab(sct.monitors[1])
        return hashlib.sha256(shot.rgb).hexdigest()


def do_act(a):
    kind = a.get("kind")
    if kind == "click":
        pyautogui.click(
            x=a["x"],
            y=a["y"],
            clicks=a.get("clicks", 1),
            button=a.get("button", "left"),
        )
    elif kind == "move":
        pyautogui.moveTo(a["x"], a["y"])
    elif kind == "type":
        pyautogui.write(a["text"], interval=a.get("interval", 0.02))
    elif kind == "key":
        pyautogui.press(a["key"])
    elif kind == "hotkey":
        pyautogui.hotkey(*a["keys"])
    elif kind == "scroll":
        pyautogui.scroll(a.get("amount", -300))
    else:
        raise ValueError(f"unknown act kind: {kind!r}")
    return {"ok": True, "kind": kind}


class H(BaseHTTPRequestHandler):
    def _send(self, code, body, ctype="application/json"):
        if isinstance(body, (dict, list)):
            body = json.dumps(body).encode()
        elif isinstance(body, str):
            body = body.encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):  # silence default stderr spam
        pass

    def do_GET(self):
        try:
            if self.path == "/health":
                w, h = pyautogui.size()
                self._send(
                    200,
                    {
                        "ok": True,
                        "version": 3,
                        "session": session_id(),
                        "screen": [w, h],
                    },
                )
            elif self.path == "/grab":
                self._send(200, grab_png(), ctype="image/png")
            elif self.path == "/grabhash":
                self._send(200, {"ok": True, "hash": grab_hash()})
            else:
                self._send(404, {"ok": False, "error": "not found"})
        except Exception as e:
            log("GET " + self.path + " ERR: " + traceback.format_exc())
            self._send(500, {"ok": False, "error": repr(e)})

    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(n) if n else b"{}"
        try:
            body = json.loads(raw or b"{}")
            if self.path == "/act":
                self._send(200, do_act(body))
            elif self.path == "/run":
                # Optional env dict is merged over the agent's env so the harness
                # can pass a known CLOTO_API_KEY to the launched app (kernel oracle
                # auth), matching the opverify daemon flow.
                env = {**os.environ, **body["env"]} if body.get("env") else None
                p = subprocess.Popen([body["path"]] + body.get("args", []), env=env)
                self._send(200, {"ok": True, "pid": p.pid})
            elif self.path == "/quit":
                self._send(200, {"ok": True, "quitting": True})
                log("quit requested")
                sys.exit(0)
            else:
                self._send(404, {"ok": False, "error": "not found"})
        except SystemExit:
            raise
        except Exception as e:
            log("POST " + self.path + " ERR: " + traceback.format_exc())
            self._send(500, {"ok": False, "error": repr(e)})


def main():
    log(f"opv_agent starting: session={session_id()} port={PORT}")
    srv = ThreadingHTTPServer(("127.0.0.1", PORT), H)
    log("listening")
    srv.serve_forever()


if __name__ == "__main__":
    main()
