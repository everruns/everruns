#!/usr/bin/env bash
set -euo pipefail
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if ! command -v caddy >/dev/null 2>&1; then
  echo "SKIP: caddy not installed"
  exit 0
fi
python3 - "$PROJECT_ROOT" <<'PY'
import http.client
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]

api, proxy, admin, ui = [port() for _ in range(4)]
hits = []
body = b'{"message":"once"}'
server = None

class Fixture(BaseHTTPRequestHandler):
    def do_POST(self):
        hits.append((self.path, self.rfile.read(int(self.headers["Content-Length"]))))
        if self.path.endswith("/ambiguous"):
            # Received the POST, then lost its response. Retrying could duplicate it.
            self.connection.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
            self.connection.close()
            return
        self.send_response(201)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b'{}')
    def log_message(self, *_):
        pass

def start_api():
    global server
    time.sleep(.2)
    server = ThreadingHTTPServer(("127.0.0.1", api), Fixture)
    server.serve_forever()

def request(target, method, path, data=None):
    c = http.client.HTTPConnection("127.0.0.1", target, timeout=8)
    try:
        c.request(method, path, body=data)
        r = c.getresponse()
        status = r.status
        r.read()
        return status
    finally:
        c.close()

p = subprocess.Popen(["caddy", "run", "--config", str(Path(sys.argv[1]) / "local/Caddyfile"), "--adapter", "caddyfile"],
    env=os.environ | {"API_PORT": str(api), "PROXY_PORT": str(proxy), "CADDY_ADMIN_PORT": str(admin), "UI_PORT": str(ui)},
    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    deadline = time.monotonic() + 5
    while True:
        try:
            if request(admin, "GET", "/config/") == 200:
                break
        except OSError:
            pass
        assert time.monotonic() < deadline, "proxy startup timed out"
        time.sleep(.02)
    threading.Thread(target=start_api, daemon=True).start()
    # No API listener yet: the first dial must fail, before sending any POST.
    assert request(proxy, "POST", "/api/v1/once", body) == 201
    assert hits == [("/api/v1/once", body)], hits
    assert request(proxy, "POST", "/api/v1/ambiguous", body) == 502
    assert hits == [("/api/v1/once", body), ("/api/v1/ambiguous", body)], hits
    print("caddy connection retry checks passed; POSTs received exactly once")
finally:
    p.terminate()
    try:
        p.wait(timeout=3)
    except subprocess.TimeoutExpired:
        p.kill()
        p.wait()
    if server:
        server.shutdown()
        server.server_close()
PY
