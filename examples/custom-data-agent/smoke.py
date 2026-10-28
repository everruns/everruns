#!/usr/bin/env python3
"""Real-model, real-control-plane custom-worker smoke. Run against a disposable stack."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import signal
import sqlite3
import subprocess
import threading
import time
import urllib.error
import urllib.request
import urllib.parse
import uuid


def require(condition, message):
    if not condition:
        raise AssertionError(message)


class Smoke:
    def __init__(self, args):
        self.args = args
        self.root = Path(args.artifacts).resolve()
        self.root.mkdir(parents=True, exist_ok=True)
        os.chmod(self.root, 0o700)
        self.headers = {"Content-Type": "application/json"}
        if os.environ.get("EVERRUNS_TOKEN"):
            self.headers["Authorization"] = "Bearer " + os.environ["EVERRUNS_TOKEN"]
        if args.org:
            self.headers["X-Org-Id"] = args.org
        self.token = secrets.token_urlsafe(32)
        self.worker = None
        self.container = "custom-data-smoke-" + uuid.uuid4().hex[:12]
        self.sse = None
        self.live = []
        self.checks = []
        self.agent = None
        self.deployment = None
        self.session = None
        self.database = self.root / "sales.sqlite"
        self.expected = {"region": "north", "orders": 2, "revenue_cents": 27278}
        self.worker_log = None
        self.image_id = None

    def check(self, message):
        self.checks.append(message)
        print("PASS " + message, flush=True)

    def url(self, path):
        base = self.args.api.rstrip("/")
        if path == "/health":
            return base.removesuffix("/api") + path
        return (base if base.endswith("/api") else base + "/api") + path

    def request(self, method, path, body=None):
        request = urllib.request.Request(self.url(path),
            data=None if body is None else json.dumps(body).encode(), headers=self.headers, method=method)
        with urllib.request.urlopen(request, timeout=20) as response:
            return json.load(response)

    def sql(self, statement, **variables):
        env = os.environ.copy()
        database = urllib.parse.urlparse(os.environ["DATABASE_URL"])
        require(database.scheme in {"postgres", "postgresql"}, "DATABASE_URL must be a PostgreSQL URI")
        env["PGCONNECT_TIMEOUT"] = "10"
        env.update(PGHOST=database.hostname or "localhost", PGPORT=str(database.port or 5432),
            PGDATABASE=urllib.parse.unquote(database.path.lstrip("/")),
            PGUSER=urllib.parse.unquote(database.username or ""), PGPASSWORD=urllib.parse.unquote(database.password or ""))
        for key, variable in {"sslmode": "PGSSLMODE", "sslrootcert": "PGSSLROOTCERT", "sslcert": "PGSSLCERT", "sslkey": "PGSSLKEY"}.items():
            value = urllib.parse.parse_qs(database.query).get(key)
            if value: env[variable] = value[0]
        command = ["psql", "-XAt", "--set=ON_ERROR_STOP=1"]
        command += ["--set=" + key + "=" + str(value) for key, value in variables.items()]
        result = subprocess.run(command, input=statement, text=True, env=env, capture_output=True, timeout=20)
        if result.returncode:
            # Never echo the connection string or operator environment.
            (self.root / "operator-error.log").write_text(result.stderr)
            raise RuntimeError("Operator SQL failed; inspect private operator-error.log")
        return result.stdout.strip().splitlines()[0] if result.stdout.strip() else ""

    def events(self):
        return self.request("GET", f"/v1/sessions/{self.session}/events?limit=1000")["data"]

    def send(self, text):
        response = self.request("POST", f"/v1/sessions/{self.session}/messages", {
            "message": {"role": "user", "content": [{"type": "text", "text": text}]}})
        return response["id"]

    def watch(self):
        # curl streams without buffering. Its config arrives over stdin so an
        # optional API credential never appears in process arguments.
        config = "".join("header = " + json.dumps(key + ": " + value) + "\n" for key, value in self.headers.items())
        self.sse = subprocess.Popen(["curl", "--config", "-", "--silent", "--show-error", "--no-buffer",
            "--max-time", "300", self.url(f"/v1/sessions/{self.session}/sse?after_sequence=0")],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.sse.stdin.write(config)
        self.sse.stdin.close()
        def collect():
            for line in self.sse.stdout:
                if line.startswith("data:"):
                    event = json.loads(line[5:].strip())
                    if event.get("type"):
                        self.live.append(event)
        threading.Thread(target=collect, daemon=True).start()

    def start_worker(self):
        # The customer process deliberately receives no CP database URL,
        # encryption key, internal fleet token, or other secret-manager values.
        keep = {"PATH", "HOME", "LD_LIBRARY_PATH", "SSL_CERT_FILE", "SSL_CERT_DIR",
            "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY", "http_proxy", "https_proxy", "no_proxy",
            "OPENAI_API_KEY", "RUST_BACKTRACE"}
        env = {key: value for key, value in os.environ.items() if key in keep}
        env.update(CUSTOM_WORKER_URL=self.args.worker_url, CUSTOM_WORKER_TOKEN=self.token,
            DATA_AGENT_DATABASE=str(self.database))
        if self.args.ca:
            env["CUSTOM_WORKER_CA_CERT"] = str(Path(self.args.ca).resolve())
        require("DATABASE_URL" not in env and "WORKER_GRPC_AUTH_TOKEN" not in env, "Worker inherited CP credentials")
        if self.args.image:
            values = {"CUSTOM_WORKER_URL": self.args.worker_url, "CUSTOM_WORKER_TOKEN": self.token,
                "OPENAI_API_KEY": os.environ["OPENAI_API_KEY"], "DATA_AGENT_DATABASE": "/data/sales.sqlite"}
            command = ["docker", "--host=unix:///var/run/docker.sock", "run", "--rm", "--name", self.container,
                "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--user=65532:65532",
                "--pids-limit=128", "--memory=512m", "--cpus=1",
                "--mount", f"type=bind,src={self.database},dst=/data/sales.sqlite,readonly"]
            if self.args.ca:
                # A public CA created under umask 077 may not be readable by
                # UID 65532. Copy it without widening the original's permissions.
                container_ca = self.root / "worker-ca.pem"
                container_ca.write_bytes(Path(self.args.ca).read_bytes())
                os.chmod(container_ca, 0o644)
                command += ["--mount", f"type=bind,src={container_ca},dst=/run/cp-ca.pem,readonly"]
                values["CUSTOM_WORKER_CA_CERT"] = "/run/cp-ca.pem"
            if self.args.proxy_ca:
                command += ["--mount", f"type=bind,src={Path(self.args.proxy_ca).resolve()},dst=/etc/ssl/certs/ca-certificates.crt,readonly"]
            # Docker resolves name-only --env options from the CLI environment:
            # secrets never appear in command arguments or a plaintext env file.
            for key in values:
                command += ["--env", key]
            command.append(self.args.image)
            env = os.environ.copy()  # Docker CLI retains the configured daemon/proxy identity.
            env.update(values)
            for key in ["DOCKER_HOST", "DOCKER_CONTEXT", "DOCKER_TLS", "DOCKER_TLS_VERIFY", "DOCKER_CERT_PATH"]:
                env.pop(key, None)
        else:
            command = [str(Path(self.args.binary).resolve())]
        self.worker_log = open(self.root / "worker.log", "a", encoding="utf-8")
        self.worker = subprocess.Popen(command, env=env, stdout=self.worker_log, stderr=subprocess.STDOUT)
        self.check("worker starts with its own model key and deployment credential only")

    def stop_worker(self):
        if self.worker is not None:
            if self.args.image:
                subprocess.run(["docker", "--host=unix:///var/run/docker.sock", "stop", "--time=5", self.container],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
            elif self.worker.poll() is None:
                self.worker.send_signal(signal.SIGINT)
            try:
                self.worker.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.worker.kill()
                self.worker.wait(timeout=5)
            self.worker = None
            self.worker_log.close()
            (self.root / "container.env").unlink(missing_ok=True)

    def verify_container(self):
        if not self.args.image:
            return
        env = os.environ.copy()
        for key in ["DOCKER_HOST", "DOCKER_CONTEXT", "DOCKER_TLS", "DOCKER_TLS_VERIFY", "DOCKER_CERT_PATH"]:
            env.pop(key, None)
        template = '{"user":{{json .Config.User}},"host":{{json .HostConfig}},"mounts":{{json .Mounts}},"image":{{json .Image}}}'
        result = subprocess.run(["docker", "--host=unix:///var/run/docker.sock", "inspect", "--format", template, self.container],
            capture_output=True, text=True, check=True, env=env)
        state = json.loads(result.stdout)
        self.image_id = state["image"]
        names_result = subprocess.run(["docker", "--host=unix:///var/run/docker.sock", "inspect", "--format",
            '{{range .Config.Env}}{{println (index (split . "=") 0)}}{{end}}', self.container],
            capture_output=True, text=True, check=True, env=env)
        names = set(names_result.stdout.split())
        require(not names.intersection({"DATABASE_URL", "WORKER_GRPC_AUTH_TOKEN", "DEFAULT_OPENAI_API_KEY", "SECRETS_ENCRYPTION_KEY"}), "Container inherited CP credentials")
        host = state["host"]
        require(state["user"] == "65532:65532" and host["ReadonlyRootfs"] and not host["Privileged"], "Container identity/filesystem boundary is wrong")
        require("ALL" in host["CapDrop"] and any(v.startswith("no-new-privileges") for v in host["SecurityOpt"]), "Container privilege boundary is wrong")
        require(host["Memory"] == 512 * 1024 * 1024 and host["PidsLimit"] == 128 and host["NetworkMode"] != "host", "Container resource/network boundary is wrong")
        require(all(not mount["RW"] for mount in state["mounts"]), "Container has a writable host mount")
        (self.root / "container-boundary.json").write_text(json.dumps({
            "user": state["user"], "read_only_root": host["ReadonlyRootfs"], "cap_drop": host["CapDrop"],
            "security_options": host["SecurityOpt"], "memory_bytes": host["Memory"], "pids_limit": host["PidsLimit"],
            "network": host["NetworkMode"], "environment_names": sorted(names), "mounts": [{"destination": m["Destination"], "read_only": not m["RW"]} for m in state["mounts"]]}, indent=2))
        self.check("container runs non-root with read-only mounts/root, dropped capabilities, bounded resources and bridge networking")

    def wait_turn(self, input_id):
        deadline = time.monotonic() + self.args.timeout
        while time.monotonic() < deadline:
            events = self.events()
            matching = [e for e in events if e.get("context", {}).get("input_message_id") == input_id]
            failed = [e for e in matching if e["type"] in {"turn.failed", "turn.cancelled"}]
            require(not failed, "Custom turn failed; inspect events.json and worker.log")
            if any(e["type"] == "turn.completed" for e in matching):
                (self.root / "events.json").write_text(json.dumps(events, indent=2))
                return matching
            if self.worker and self.worker.poll() is not None:
                raise RuntimeError("Worker exited before completing input; inspect worker.log")
            (self.root / "events.json").write_text(json.dumps(events, indent=2))
            time.sleep(0.4)
        raise TimeoutError("Custom turn did not complete before the deadline")

    def verify_turn(self, events):
        kinds = {e["type"] for e in events}
        for kind in ["turn.started", "turn.completed", "llm.generation", "tool.started", "tool.completed", "output.message.completed"]:
            require(kind in kinds, "Missing canonical event: " + kind)
        results = [json.loads("".join(p.get("text", "") for p in e["data"].get("result", [])))
            for e in events if e["type"] == "tool.completed" and e["data"]["tool_name"] == "sales_summary" and e["data"]["success"]]
        require(self.expected in results, "Tool result does not match the private database")
        answers = [e["data"]["message"] for e in events if e["type"] == "output.message.completed"]
        text = " ".join(p.get("text", "") for answer in answers for p in answer["content"]).replace(",", "")
        require(str(self.expected["revenue_cents"]) in text, "Answer did not report the database total")
        execution = [e for e in events if not e["type"].startswith(("session.", "input."))]
        require(all(e.get("metadata", {}).get("custom_worker_id") == self.deployment for e in execution), "Execution provenance missing")
        turns = {e["context"]["turn_id"] for e in execution}
        require(len(turns) == 1, "Turn correlation diverged")
        require(all(len(e["context"].get("trace_id", "")) == 32 for e in execution), "Trace correlation missing")
        for event in events:
            if event["type"] == "llm.generation":
                require(not event["data"]["metadata"].get("provider_id"), "Custom usage claims a CP provider")
        require(self.request("GET", f"/v1/sessions/{self.session}")["status"] == "idle", "Session status did not become idle")
        messages = self.request("GET", f"/v1/sessions/{self.session}/messages")["data"]
        require(any(m["role"] == "agent" for m in messages), "Ordinary messages API has no assistant answer")
        work = self.request("GET", f"/v1/sessions/{self.session}/work")
        (self.root / "work-report.json").write_text(json.dumps(work, indent=2))
        require(work.get("report"), "Work/trace report is empty")
        self.check("real model calls private sales tool; answer, usage, trace, messages and idle status reach the CP")

    def run(self):
        require(os.environ.get("OPENAI_API_KEY"), "Supply OPENAI_API_KEY from your secret manager")
        require(os.environ.get("DATABASE_URL"), "Operator provisioning needs DATABASE_URL")
        self.request("GET", "/health")
        active = int(self.sql("SELECT count(*) FROM durable_workers WHERE last_heartbeat_at > NOW() - INTERVAL '30 seconds' AND accepting_tasks"))
        require(active > 0, "Smoke requires a live built-in worker to prove routing isolation")
        self.check("control plane and built-in worker are healthy")
        self.agent = self.request("POST", "/v1/agents", {"name": "custom-smoke-" + uuid.uuid4().hex[:10],
            "system_prompt": "The custom worker owns this agent's implementation."})["id"]
        self.deployment = self.sql("""INSERT INTO custom_workers (org_id, agent_id, name, token_hash)
            SELECT org_id, id, 'smoke-data-agent', decode(:'digest', 'hex') FROM agents WHERE public_id = :'agent'
            RETURNING id;""", agent=self.agent, digest=hashlib.sha256(self.token.encode()).hexdigest())
        require(self.deployment, "Provisioning created no binding")
        with sqlite3.connect(self.database) as db:
            db.executescript("CREATE TABLE sales(region TEXT NOT NULL, amount_cents INTEGER NOT NULL); INSERT INTO sales VALUES ('north',18037),('north',9241),('south',7000);")
        os.chmod(self.database, 0o644)
        self.session = self.request("POST", "/v1/sessions", {"agent_id": self.agent, "source": "playground", "title": "Custom data worker smoke"})["id"]
        (self.root / "session.json").write_text(json.dumps({"agent_id": self.agent, "session_id": self.session, "deployment_id": self.deployment}, indent=2))
        self.watch()
        question = "Use sales_summary for north. Report orders and revenue_cents as integers in JSON. Do not guess."
        first = self.send(question)
        time.sleep(2)
        require(not any(e["type"] == "turn.started" for e in self.events()), "Built-in worker stole custom input")
        self.check("input stays queued while the custom worker is absent, despite an active built-in worker")
        self.start_worker()
        self.verify_turn(self.wait_turn(first))
        self.verify_container()
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and not any(e["type"] == "output.message.delta" for e in self.live):
            time.sleep(0.1)
        require(any(e["type"] == "output.message.delta" for e in self.live), "SSE delivered no live output delta")
        require(any(e["type"] == "tool.completed" for e in self.live), "SSE delivered no tool result")
        self.check("ordinary SSE delivers live model output and tool events")
        # Kill the process/container and start a new replica; history stays in CP.
        self.stop_worker()
        with sqlite3.connect(self.database) as db:
            db.execute("INSERT INTO sales VALUES ('north',1234)")
        self.expected.update(orders=3, revenue_cents=28512)
        second = self.send("Query the same region again with sales_summary. Return fresh orders and revenue_cents as integer JSON.")
        self.start_worker()
        self.verify_turn(self.wait_turn(second))
        self.check("new replica reads CP history and observes changed private database contents")
        self.stop_worker()
        self.sql("UPDATE custom_workers SET enabled = FALSE WHERE id = :'deployment'::uuid", deployment=self.deployment)
        third = self.send(question)
        self.start_worker()
        self.worker.wait(timeout=15)
        require(self.worker.returncode != 0, "Revoked credential was accepted")
        time.sleep(2)
        require(not any(e["type"] == "turn.started" and e.get("context", {}).get("input_message_id") == third for e in self.events()), "Revoked binding fell back to built-in execution")
        self.check("revocation rejects the worker and preserves queued custom routing")
        self.stop_worker()
        self.token = secrets.token_urlsafe(32)
        self.sql("UPDATE custom_workers SET enabled = TRUE, token_hash = decode(:'digest','hex') WHERE id = :'deployment'::uuid",
            digest=hashlib.sha256(self.token.encode()).hexdigest(), deployment=self.deployment)
        self.start_worker()
        self.verify_turn(self.wait_turn(third))
        self.check("rotated credential completes the original queued input")
        self.stop_worker()
        events = self.events()
        sequences = [e["sequence"] for e in events]
        require(sequences == sorted(set(sequences)), "Durable event sequences are not unique and ordered")
        require(len([e for e in events if e["type"] == "turn.completed"]) == 3, "Unexpected duplicate terminal events")
        report = {"status": "passed", "mode": "container" if self.args.image else "process",
            "worker_tls": self.args.worker_url.startswith("https://"), "worker_image_id": self.image_id,
            "agent_id": self.agent, "session_id": self.session, "deployment_id": self.deployment,
            "checks": self.checks, "durable_events": len(events), "sse_events": len(self.live), "completed_turns": 3}
        (self.root / "report.json").write_text(json.dumps(report, indent=2))
        (self.root / "sse-events.json").write_text(json.dumps(self.live, indent=2))
        print("Smoke passed; evidence: " + str(self.root / "report.json"), flush=True)

    def close(self):
        try:
            self.stop_worker()
        finally:
            try:
                if self.sse:
                    self.sse.terminate()
                    self.sse.wait(timeout=5)
            finally:
                try:
                    # Revoke even if container shutdown or the SSE reader failed.
                    if self.deployment:
                        self.sql("UPDATE custom_workers SET enabled = FALSE WHERE id = :'deployment'::uuid", deployment=self.deployment)
                finally:
                    (self.root / "container.env").unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api", required=True, help="Control plane URL, e.g. http://127.0.0.1:28701")
    parser.add_argument("--worker-url", required=True, help="Custom gRPC URL reachable by the worker")
    parser.add_argument("--binary", default="target/debug/everruns-custom-data-agent")
    parser.add_argument("--image", help="Run a Docker worker instead of a local process")
    parser.add_argument("--ca", help="Custom listener/ingress CA PEM")
    parser.add_argument("--proxy-ca", help="System CA bundle to mount read-only in the worker container")
    parser.add_argument("--org", help="Public organization id when API auth needs an explicit selector")
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--artifacts", default="work/custom-worker-smoke-" + uuid.uuid4().hex[:8])
    smoke = Smoke(parser.parse_args())
    try:
        smoke.run()
    except Exception as error:
        (smoke.root / "report.json").write_text(json.dumps({"status": "failed", "error_type": type(error).__name__,
            "error": str(error), "checks": smoke.checks, "session_id": smoke.session}, indent=2))
        raise
    finally:
        smoke.close()


if __name__ == "__main__":
    main()
