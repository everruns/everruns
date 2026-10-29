"""Safety regressions for the live smoke client; no external services."""
import http.server
from pathlib import Path
import subprocess
import sys
import threading
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "smoke.py"


class SmokeClientTest(unittest.TestCase):
    def run_smoke(self, cp):
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--cp", cp, "--endpoint", "https://agent.example/"],
            env={"EVERRUNS_TOKEN": "synthetic-auth", "EVERRUNS_ENDPOINT_SECRET": "synthetic-endpoint-secret-of-sufficient-length"},
            text=True, capture_output=True, timeout=10,
        )

    def test_public_plaintext_cp_is_rejected_before_credentials_are_used(self):
        result = self.run_smoke("http://example.com")
        self.assertEqual(result.returncode, 2)
        self.assertIn("requires HTTPS", result.stderr)
        self.assertNotIn("synthetic-auth", result.stderr)

    def test_redirect_never_forwards_the_user_credential(self):
        requests = []

        class Receiver(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                requests.append(self.headers.get("Authorization"))
                self.send_response(200)
                self.end_headers()
            def log_message(self, *_args):
                pass

        receiver = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Receiver)

        class Redirect(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(302)
                self.send_header("Location", f"http://127.0.0.1:{receiver.server_port}/sink")
                self.end_headers()
            def log_message(self, *_args):
                pass

        redirect = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Redirect)
        for server in (receiver, redirect):
            threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            result = self.run_smoke(f"http://127.0.0.1:{redirect.server_port}")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("HTTP 302", result.stderr)
            self.assertEqual(requests, [])
            self.assertNotIn("synthetic-auth", result.stderr)
        finally:
            for server in (receiver, redirect):
                server.shutdown()
                server.server_close()


if __name__ == "__main__":
    unittest.main()
