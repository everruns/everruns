#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

if ! command -v caddy >/dev/null 2>&1; then
  echo "SKIP: caddy not installed"
  exit 0
fi

python3 - "$PROJECT_ROOT" <<'PY'
from __future__ import annotations

import gzip
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


PROJECT_ROOT = Path(sys.argv[1])
CAPABILITIES_BODY = b'{"data":"' + (b"a" * (232_755 - 11)) + b'"}'
ERROR_BODY = json.dumps({"error": "invalid request", "detail": "x" * 2_048}).encode()
ALREADY_COMPRESSED_BODY = gzip.compress(b'{"data":"' + (b"b" * 2_048) + b'"}')
FIRST_EVENT_SENT = threading.Event()
FINISH_STREAM = threading.Event()


class ApiFixture(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self) -> None:
        if self.path == "/api/v1/capabilities":
            self._send(200, "application/json", CAPABILITIES_BODY)
        elif self.path == "/api/v1/small":
            self._send(200, "application/json", b'{"ok":true}')
        elif self.path == "/api/v1/already-compressed":
            self._send(
                200,
                "application/json",
                ALREADY_COMPRESSED_BODY,
                {"Content-Encoding": "gzip"},
            )
        elif self.path == "/api/v1/error":
            self._send(400, "application/json", ERROR_BODY)
        elif self.path == "/api/v1/events":
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.end_headers()
            self.wfile.write(b"event: first\ndata: {}\n\n")
            self.wfile.flush()
            FIRST_EVENT_SENT.set()
            FINISH_STREAM.wait(timeout=5)
            self.wfile.write(b"event: second\ndata: {}\n\n")
            self.wfile.flush()
        else:
            self._send(404, "application/json", b'{"error":"not found"}')

    def _send(
        self,
        status: int,
        content_type: str,
        body: bytes,
        headers: dict[str, str] | None = None,
    ) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args: object) -> None:
        pass


class QuietThreadingHTTPServer(ThreadingHTTPServer):
    def handle_error(self, _request: object, _client_address: object) -> None:
        pass


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def request(port: int, path: str, accept_encoding: str = "gzip") -> tuple[int, dict[str, str], bytes]:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
    connection.request("GET", path, headers={"Accept-Encoding": accept_encoding})
    response = connection.getresponse()
    result = response.status, {name.lower(): value for name, value in response.getheaders()}, response.read()
    connection.close()
    return result


api_port, proxy_port, admin_port, ui_port = (free_port() for _ in range(4))
server = QuietThreadingHTTPServer(("127.0.0.1", api_port), ApiFixture)
server_thread = threading.Thread(target=server.serve_forever, daemon=True)
server_thread.start()

environment = os.environ.copy()
environment.update(
    API_PORT=str(api_port),
    PROXY_PORT=str(proxy_port),
    CADDY_ADMIN_PORT=str(admin_port),
    UI_PORT=str(ui_port),
)
caddy = subprocess.Popen(
    ["caddy", "run", "--config", str(PROJECT_ROOT / "local/Caddyfile"), "--adapter", "caddyfile"],
    env=environment,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.PIPE,
    text=True,
)

try:
    deadline = time.monotonic() + 5
    while True:
        if caddy.poll() is not None:
            raise AssertionError(f"caddy exited during startup: {caddy.stderr.read()}")
        try:
            request(proxy_port, "/api/v1/small", "identity")
            break
        except OSError:
            if time.monotonic() >= deadline:
                raise AssertionError("caddy did not become ready")
            time.sleep(0.05)

    status, headers, compressed_body = request(proxy_port, "/api/v1/capabilities")
    assert status == 200
    assert headers.get("content-encoding") == "gzip", headers
    assert "accept-encoding" in headers.get("vary", "").lower(), headers
    assert gzip.decompress(compressed_body) == CAPABILITIES_BODY
    assert len(compressed_body) < len(CAPABILITIES_BODY) // 10

    _, small_headers, small_body = request(proxy_port, "/api/v1/small")
    assert "content-encoding" not in small_headers, small_headers
    assert small_body == b'{"ok":true}'

    _, encoded_headers, encoded_body = request(proxy_port, "/api/v1/already-compressed")
    assert encoded_headers.get("content-encoding") == "gzip", encoded_headers
    assert encoded_body == ALREADY_COMPRESSED_BODY
    assert gzip.decompress(encoded_body).startswith(b'{"data":"')

    error_status, error_headers, error_body = request(proxy_port, "/api/v1/error")
    assert error_status == 400
    assert error_headers.get("content-encoding") == "gzip", error_headers
    assert json.loads(gzip.decompress(error_body))["error"] == "invalid request"

    connection = http.client.HTTPConnection("127.0.0.1", proxy_port, timeout=2)
    connection.request("GET", "/api/v1/events", headers={"Accept-Encoding": "gzip"})
    response = connection.getresponse()
    assert response.status == 200
    sse_headers = {name.lower(): value for name, value in response.getheaders()}
    assert "content-encoding" not in sse_headers, sse_headers
    started = time.monotonic()
    assert response.readline() == b"event: first\n"
    assert time.monotonic() - started < 1
    assert FIRST_EVENT_SENT.is_set()
    assert not FINISH_STREAM.is_set()
    FINISH_STREAM.set()
    assert response.readline() == b"data: {}\n"
    connection.close()

    print(
        "caddy API compression checks passed "
        f"(capabilities wire bytes: {len(CAPABILITIES_BODY)} -> {len(compressed_body)})"
    )
finally:
    FINISH_STREAM.set()
    caddy.terminate()
    try:
        caddy.wait(timeout=3)
    except subprocess.TimeoutExpired:
        caddy.kill()
        caddy.wait(timeout=3)
    server.shutdown()
    server.server_close()
PY
