"""Safety regressions for the live smoke client; no external services."""
import http.server
import json
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

    def test_registration_retries_lease_conflicts_but_not_permission_denials(self):
        for first_status, expected_attempts in ((409, 2), (403, 1)):
            with self.subTest(status=first_status):
                attempts = []

                class ControlPanel(http.server.BaseHTTPRequestHandler):
                    def reply(self, status, body):
                        self.send_response(status)
                        self.send_header("Content-Type", "application/json")
                        self.end_headers()
                        self.wfile.write(json.dumps(body).encode())
                    def do_GET(self):
                        self.reply(200, {"data": [{"id": "harness_fixture", "name": "conversation"}]})
                    def do_POST(self):
                        self.rfile.read(int(self.headers.get("Content-Length", 0)))
                        if self.path.endswith("/external-worker"):
                            attempts.append(True)
                            self.reply(first_status if len(attempts) == 1 else 200,
                                       {"detail": "Lease remains active", "enabled": True})
                        elif self.path.endswith("/agents"):
                            self.reply(200, {"id": "agent_fixture"})
                        else:
                            self.reply(400, {"detail": "End fixture after registration"})
                    def do_DELETE(self):
                        self.reply(200, None)
                    def log_message(self, *_args):
                        pass

                server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ControlPanel)
                threading.Thread(target=server.serve_forever, daemon=True).start()
                try:
                    result = self.run_smoke(f"http://127.0.0.1:{server.server_port}")
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(len(attempts), expected_attempts)
                    self.assertNotIn("synthetic-auth", result.stderr)
                finally:
                    server.shutdown()
                    server.server_close()

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
