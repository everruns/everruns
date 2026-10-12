#!/usr/bin/env bash
# Chaos harness (scripts/chaos/) setup guards, hermetic: no binaries, no infra.
set -euo pipefail
cd "$(dirname "$0")/.."
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import http.server
import json
import sys
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, "scripts/chaos")
import topology  # noqa: E402
from load import Driver, SessionRun  # noqa: E402


class Health(http.server.BaseHTTPRequestHandler):
    backend = None

    def do_GET(self):
        if self.path != "/api/v1/durable/health":
            self.send_error(404)
            return
        body = json.dumps({"status": "healthy", "event_delivery": self.backend}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


def health_server(backend):
    handler = type("H", (Health,), {"backend": backend})
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def topo_with(backends):
    args = SimpleNamespace(
        port_prefix="434", database_url="postgres://x", nats_url="nats://localhost:43422",
        valkey_url="redis://x", rust_log="info", servers=len(backends), workers=1,
        worker_slots=1, server_bin="/bin/true", worker_bin="/bin/true", secrets_key="k",
    )
    topo = topology.Topology(args, "/nonexistent")
    servers = [health_server(b) for b in backends]
    for proc, srv in zip(topo.servers, servers):
        proc.http_port = srv.server_address[1]
    return topo, servers


class SharedEventDeliveryTests(unittest.TestCase):
    # CI's NATS once ran without JetStream: each server fell back to
    # in-process delivery and an approval session hung, reported as a lost turn.
    def test_in_memory_server_is_refused(self):
        topo, servers = topo_with(["nats", "in_memory"])
        try:
            with self.assertRaisesRegex(RuntimeError, r"server-1=in_memory.*JetStream"):
                topo.require_shared_event_delivery()
        finally:
            for s in servers:
                s.shutdown()
                s.server_close()

    def test_all_nats_is_accepted(self):
        topo, servers = topo_with(["nats", "nats"])
        try:
            topo.require_shared_event_delivery()
            self.assertEqual(topo.event_delivery(), {"server-0": "nats", "server-1": "nats"})
        finally:
            for s in servers:
                s.shutdown()
                s.server_close()

    def test_unreachable_health_counts_as_unshared(self):
        self.assertEqual(topology.unshared_event_delivery({"server-0": None}), ["server-0=unknown"])
        self.assertEqual(topology.unshared_event_delivery({"server-0": "nats"}), [])


class WorkflowTests(unittest.TestCase):
    def test_ci_nats_runs_jetstream(self):
        wf = Path(".github/workflows/chaos.yml").read_text()
        services = wf.split("services:", 1)[1].split("steps:", 1)[0]
        self.assertNotIn("nats:", services, "a service container cannot pass --jetstream")
        self.assertRegex(wf, r"docker run[^\n]*chaos-nats[^\n]*\\\n[^\n]*nats:[^\n]*--jetstream")


class ApprovalDriverTests(unittest.TestCase):
    def test_unseen_approval_request_is_reported(self):
        class Api:
            def post(self, path, body=None):
                return {"id": "message_1", "delivery": "started"}

        class Stream:
            events = []

            def wait_for(self, pred, timeout):
                return None

        driver = Driver(Api(), SimpleNamespace(), {}, {})
        run = SessionRun(index=0, kind="approval", session_id="session_1")
        driver.run_approval(run, Stream(), 0, 0.01)
        self.assertEqual(len(run.errors), 1)
        self.assertIn("approval: no pending request", run.errors[0])


unittest.main(verbosity=2)
PY
