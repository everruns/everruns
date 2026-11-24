#!/usr/bin/env bash
# The published-image smokes must select a namespace before any credentials or writes.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python3 - "$ROOT/scripts/lib/smoke-turn.sh" <<'PY'
import http.server
import subprocess
import sys
import threading
import unittest


class NamespaceSelection(unittest.TestCase):
    def select(self, responses):
        requests = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                requests.append((self.path, self.headers.get("Authorization")))
                self.send_response(responses.get(self.path, 404))
                self.end_headers()

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        base = f"http://127.0.0.1:{server.server_port}"
        try:
            result = subprocess.run(
                ["bash", "-c", 'source "$1"; smoke_management_api_url "$2"',
                 "smoke-test", sys.argv[1], base],
                capture_output=True, text=True, timeout=10,
            )
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
        return base, result, requests

    def test_prefers_canonical_without_credentials(self):
        base, result, requests = self.select({"/api/v1/platform/auth/config": 200})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), base + "/api/v1/platform")
        self.assertEqual(requests, [("/api/v1/platform/auth/config", None)])

    def test_supports_published_legacy_images(self):
        base, result, requests = self.select({"/api/v1/auth/config": 200})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), base + "/api/v1")
        self.assertEqual(requests, [("/api/v1/platform/auth/config", None),
                                    ("/api/v1/auth/config", None)])

    def test_denials_and_server_errors_never_change_namespace(self):
        for status in (401, 403, 500):
            with self.subTest(status=status):
                _, result, requests = self.select({"/api/v1/platform/auth/config": status,
                                                    "/api/v1/auth/config": 200})
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(requests, [("/api/v1/platform/auth/config", None)])

    def test_missing_both_namespaces_fails(self):
        _, result, requests = self.select({})
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(requests), 2)


unittest.main(argv=[sys.argv[0]])
PY
