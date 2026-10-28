"""Smoke harness regressions: API routing and secret/CA deployment boundaries."""
import argparse
import importlib.util
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("custom_worker_smoke", Path(__file__).parents[1] / "smoke.py")
smoke_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke_module)


def args(root, **overrides):
    values = dict(artifacts=str(root), api="http://127.0.0.1:1", org=None, image=None,
        ca=None, proxy_ca=None, binary="target/debug/everruns-custom-data-agent",
        worker_url="http://127.0.0.1:9002", timeout=1)
    values.update(overrides)
    return argparse.Namespace(**values)


class SmokeContracts(unittest.TestCase):
    def test_api_namespace_and_health_match_real_server_mounts(self):
        paths = []
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                paths.append(self.path)
                body = json.dumps({"ok": self.path in {"/health", "/api/v1/agents"}}).encode()
                self.send_response(200 if self.path in {"/health", "/api/v1/agents"} else 404)
                self.end_headers()
                self.wfile.write(body)
            def log_message(self, *_args):
                pass
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as root:
                for suffix in ["", "/api", "/api/"]:
                    smoke = smoke_module.Smoke(args(root, api=f"http://127.0.0.1:{server.server_port}" + suffix))
                    self.assertTrue(smoke.request("GET", "/health")["ok"])
                    self.assertTrue(smoke.request("GET", "/v1/agents")["ok"])
            self.assertEqual(paths, ["/health", "/api/v1/agents"] * 3)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def test_process_worker_never_inherits_operator_credentials(self):
        with tempfile.TemporaryDirectory() as root, patch.dict(os.environ, {
            "DATABASE_URL": "postgresql://operator:private@localhost/cp",
            "WORKER_GRPC_AUTH_TOKEN": "private-fleet-token", "SECRETS_ENCRYPTION_KEY": "private-cp-key",
            "DEFAULT_OPENAI_API_KEY": "private-cp-model-key", "OPENAI_API_KEY": "application-model-key",
        }):
            smoke = smoke_module.Smoke(args(root))
            with patch.object(smoke_module.subprocess, "Popen") as popen:
                smoke.start_worker()
                env = popen.call_args.kwargs["env"]
                self.assertEqual(env["OPENAI_API_KEY"], "application-model-key")
                self.assertEqual(env["CUSTOM_WORKER_TOKEN"], smoke.token)
                for key in ["DATABASE_URL", "WORKER_GRPC_AUTH_TOKEN", "SECRETS_ENCRYPTION_KEY", "DEFAULT_OPENAI_API_KEY"]:
                    self.assertNotIn(key, env)
            smoke.worker_log.close()

    def test_revocation_and_secret_file_cleanup_survive_a_worker_shutdown_failure(self):
        with tempfile.TemporaryDirectory() as root:
            smoke = smoke_module.Smoke(args(root))
            smoke.deployment = "00000000-0000-4000-8000-000000000001"
            secret = smoke.root / "container.env"
            secret.write_text("CUSTOM_WORKER_TOKEN=synthetic-test-token\n")
            secret.chmod(0o600)
            with patch.object(smoke, "stop_worker", side_effect=RuntimeError("simulated shutdown failure")), patch.object(smoke, "sql") as sql:
                with self.assertRaisesRegex(RuntimeError, "shutdown failure"):
                    smoke.close()
                self.assertIn("enabled = FALSE", sql.call_args.args[0])
                self.assertEqual(sql.call_args.kwargs["deployment"], smoke.deployment)
                self.assertFalse(secret.exists())

    def test_non_root_container_gets_readable_public_ca_without_widening_original(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {"OPENAI_API_KEY": "application-model-key"}):
            root = Path(temporary)
            original = root / "operator-ca.pem"
            original.write_text("public CA fixture")
            original.chmod(0o600)
            smoke = smoke_module.Smoke(args(root / "evidence", image="worker:test", ca=str(original)))
            with patch.object(smoke_module.subprocess, "Popen") as popen:
                smoke.start_worker()
                command = popen.call_args.args[0]
                self.assertIn("--read-only", command)
                self.assertIn("--cap-drop=ALL", command)
                self.assertIn("--user=65532:65532", command)
            copy = smoke.root / "worker-ca.pem"
            self.assertEqual(original.stat().st_mode & 0o777, 0o600)
            self.assertEqual(copy.stat().st_mode & 0o777, 0o644)
            self.assertEqual(copy.read_bytes(), original.read_bytes())
            self.assertEqual((smoke.root / "container.env").stat().st_mode & 0o777, 0o600)
            smoke.worker_log.close()


if __name__ == "__main__":
    unittest.main()
