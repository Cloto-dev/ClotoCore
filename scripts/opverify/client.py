"""HTTP client for the ClotoCore kernel admin API (stdlib only).

The kernel wraps every JSON response in a ``{"data": ...}`` envelope; the
client unwraps it and returns the inner value. Admin routes are gated by an
``X-API-Key`` header. SSE routes (``/api/events``) additionally accept the
key as a ``?token=`` query parameter.
"""

from __future__ import annotations

import json
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Iterator, Optional


class ApiError(RuntimeError):
    """Raised when the kernel returns a non-2xx status."""

    def __init__(self, method: str, path: str, status: int, body: str):
        self.method = method
        self.path = path
        self.status = status
        self.body = body
        super().__init__(f"{method} {path} -> HTTP {status}: {body[:400]}")


class ClotoClient:
    """Thin JSON client over the kernel HTTP API.

    Parameters
    ----------
    base_url:
        e.g. ``http://127.0.0.1:8099`` (no trailing slash required).
    api_key:
        the ``CLOTO_API_KEY`` the daemon was booted with; sent as
        ``X-API-Key`` on authed calls.
    timeout:
        per-request timeout in seconds.
    """

    def __init__(self, base_url: str, api_key: str, timeout: float = 30.0):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.timeout = timeout

    # -- low level -------------------------------------------------------
    def _url(self, path: str, params: Optional[dict] = None) -> str:
        url = self.base_url + path
        if params:
            url += "?" + urllib.parse.urlencode(params)
        return url

    def request_raw(
        self,
        method: str,
        path: str,
        body: Optional[Any] = None,
        auth: bool = True,
        params: Optional[dict] = None,
        timeout: Optional[float] = None,
    ) -> tuple[int, str]:
        """Perform a request, returning ``(status, raw_text)`` without raising
        on HTTP error status. Used for negative/auth checks."""
        data = None
        headers = {"Accept": "application/json"}
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"
        if auth:
            headers["X-API-Key"] = self.api_key
        req = urllib.request.Request(
            self._url(path, params), data=data, headers=headers, method=method
        )
        try:
            with urllib.request.urlopen(req, timeout=timeout or self.timeout) as resp:
                return resp.status, resp.read().decode("utf-8", "replace")
        except urllib.error.HTTPError as e:
            return e.code, e.read().decode("utf-8", "replace")

    def request(
        self,
        method: str,
        path: str,
        body: Optional[Any] = None,
        auth: bool = True,
        params: Optional[dict] = None,
        timeout: Optional[float] = None,
    ) -> Any:
        """Perform a request and return the unwrapped ``data`` payload.

        Raises :class:`ApiError` on non-2xx.
        """
        status, text = self.request_raw(method, path, body, auth, params, timeout)
        if not (200 <= status < 300):
            raise ApiError(method, path, status, text)
        if not text:
            return None
        try:
            parsed = json.loads(text)
        except json.JSONDecodeError:
            return text
        if isinstance(parsed, dict) and "data" in parsed and len(parsed) == 1:
            return parsed["data"]
        return parsed

    # -- verb helpers ----------------------------------------------------
    def get(self, path: str, **kw) -> Any:
        return self.request("GET", path, **kw)

    def post(self, path: str, body: Optional[Any] = None, **kw) -> Any:
        return self.request("POST", path, body=body, **kw)

    def delete(self, path: str, **kw) -> Any:
        return self.request("DELETE", path, **kw)

    # -- readiness -------------------------------------------------------
    def wait_healthy(self, timeout: float = 60.0, interval: float = 0.5) -> None:
        """Poll ``GET /api/system/health`` until it reports ``status == ok``.

        Health is a no-auth route. Raises TimeoutError if never ready.
        """
        deadline = time.monotonic() + timeout
        last_err: Optional[str] = None
        while time.monotonic() < deadline:
            try:
                data = self.get("/api/system/health", auth=False, timeout=3.0)
                if isinstance(data, dict) and data.get("status") == "ok":
                    return
                last_err = f"unexpected health body: {data!r}"
            except (urllib.error.URLError, ApiError, OSError) as e:
                last_err = str(e)
            time.sleep(interval)
        raise TimeoutError(f"kernel not healthy within {timeout}s ({last_err})")

    # -- SSE -------------------------------------------------------------
    def sse(self, path: str = "/api/events", timeout: float = 15.0) -> Iterator[dict]:
        """Yield decoded SSE events from an event-stream route.

        The key is passed as ``?token=`` since EventSource cannot set
        headers. Each yielded item is the JSON-decoded ``data:`` payload
        (non-JSON payloads are yielded as ``{"raw": <text>}``). The
        generator stops when ``timeout`` seconds elapse with no new event
        or the stream closes.
        """
        req = urllib.request.Request(
            self._url(path, {"token": self.api_key}),
            headers={"Accept": "text/event-stream"},
            method="GET",
        )
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            buf: list[str] = []
            for raw_line in resp:
                line = raw_line.decode("utf-8", "replace").rstrip("\n")
                if line == "":
                    # dispatch accumulated event
                    data_lines = [
                        ln[5:].lstrip() for ln in buf if ln.startswith("data:")
                    ]
                    buf = []
                    if not data_lines:
                        continue
                    payload = "\n".join(data_lines)
                    try:
                        yield json.loads(payload)
                    except json.JSONDecodeError:
                        yield {"raw": payload}
                else:
                    buf.append(line)
