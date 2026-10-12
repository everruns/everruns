"""Minimal stdlib HTTP client for the public Everruns API.

Requests go to one base URL picked round robin by the caller. A connection
error (the server was SIGKILLed) is retried on the next server; that is what a
client behind a load balancer sees. Writes that must not double-apply carry
`client_message_id`, so a retry after a lost response is deduplicated by the
server rather than by the harness.
"""

from __future__ import annotations

import http.client
import itertools
import json
import threading
import time
import urllib.parse


class ApiError(Exception):
    def __init__(self, status: int, body: str, method: str, path: str):
        super().__init__(f"{method} {path} -> {status}: {body[:300]}")
        self.status = status
        self.body = body


class Api:
    def __init__(self, base_urls: list[str], timeout: float = 30.0, prefix: str = "/api"):
        # The server mounts the public API under /api (Caddy forwards it as is).
        self.prefix = prefix
        self.base_urls = base_urls
        self.timeout = timeout
        self._rr = itertools.count()
        self._lock = threading.Lock()
        self.request_count = 0
        self.conn_errors = 0
        self.retries = 0

    def _pick(self, offset: int = 0) -> str:
        with self._lock:
            n = next(self._rr)
        return self.base_urls[(n + offset) % len(self.base_urls)]

    def request(
        self,
        method: str,
        path: str,
        body=None,
        query: dict | None = None,
        base: str | None = None,
        retries: int = 8,
        timeout: float | None = None,
        retry_5xx: bool = True,
    ):
        path = self.prefix + path
        if query:
            path = path + "?" + urllib.parse.urlencode(query, doseq=True)
        data = json.dumps(body).encode() if body is not None else None
        last_exc: Exception | None = None
        for attempt in range(retries + 1):
            url = base if (base and attempt == 0) else self._pick(attempt)
            parsed = urllib.parse.urlparse(url)
            conn = http.client.HTTPConnection(
                parsed.hostname, parsed.port, timeout=timeout or self.timeout
            )
            try:
                headers = {"Accept": "application/json"}
                if data is not None:
                    headers["Content-Type"] = "application/json"
                conn.request(method, path, body=data, headers=headers)
                resp = conn.getresponse()
                raw = resp.read().decode("utf-8", "replace")
                with self._lock:
                    self.request_count += 1
                if resp.status >= 500 and retry_5xx and attempt < retries:
                    last_exc = ApiError(resp.status, raw, method, path)
                    with self._lock:
                        self.retries += 1
                    time.sleep(min(0.2 * (attempt + 1), 2.0))
                    continue
                if resp.status >= 400:
                    raise ApiError(resp.status, raw, method, path)
                return json.loads(raw) if raw else None
            except (ConnectionError, OSError, http.client.HTTPException) as e:
                last_exc = e
                with self._lock:
                    self.conn_errors += 1
                    self.retries += 1
                time.sleep(min(0.25 * (attempt + 1), 2.0))
            finally:
                conn.close()
        raise RuntimeError(f"{method} {path} failed after retries: {last_exc}")

    # --- convenience -------------------------------------------------------

    def get(self, path, **kw):
        return self.request("GET", path, **kw)

    def post(self, path, body=None, **kw):
        return self.request("POST", path, body=body, **kw)

    def events(self, session_id: str, after_sequence: int | None = None) -> list[dict]:
        """All persisted events of a session, oldest first, paging forward."""
        out: list[dict] = []
        cursor = after_sequence if after_sequence is not None else 0
        while True:
            page = self.get(
                f"/v1/sessions/{session_id}/events",
                query={"after_sequence": cursor, "limit": 1000},
            )
            items = page.get("data", page) if isinstance(page, dict) else page
            if not items:
                break
            for e in items:
                e.setdefault("event_type", e.get("type"))
            out.extend(items)
            seqs = [e.get("sequence") for e in items if e.get("sequence") is not None]
            if not seqs or len(items) < 1000:
                break
            cursor = max(seqs)
        return out
