"""Disposable local capacity topology. Setup is excluded from timed samples.

No production target, real provider credential, unsigned identity or auth bypass
is accepted. Interactive sign-in is excluded; the fixture uses production JWT
minting/storage and the BFF's encrypted opaque-session format.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import platform
import secrets
import socket
import subprocess
import sys
import time
import uuid

import httpx

ROOT = Path(__file__).resolve().parents[2]
PROVIDER_PREFIXES = (
    "OPENAI_",
    "ANTHROPIC_",
    "GEMINI_",
    "OPENROUTER_",
    "MISTRAL_",
    "GROQ_",
    "FIREWORKS_",
    "AWS_",
    "DEFAULT_",
    "UTILITY_",
    "BRAINTRUST_",
    "OTEL_EXPORTER_",
)


def clean_environment(source):
    # Keep platform proxy/auth guard variables. These processes only reach
    # fixture loopback services, and receive no LLM/telemetry credentials.
    return {
        k: v
        for k, v in source.items()
        if not k.startswith(PROVIDER_PREFIXES)
        and not k.endswith("_API_KEY")
        and k not in {"DOPPLER_TOKEN", "GITHUB_TOKEN", "GH_TOKEN", "LINEAR_API_KEY"}
    }


def private_json(path, value):
    with open(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as file:
        json.dump(value, file)


def atomic_json(path, value):
    path = Path(path)
    temporary = path.with_name(path.name + ".tmp")
    private_json(temporary, value)
    temporary.replace(path)


def hardware():
    memory = (
        os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")
        if sys.platform == "linux"
        else int(subprocess.check_output(["sysctl", "-n", "hw.memsize"]))
    )
    quota = Path("/sys/fs/cgroup/cpu.max")
    cpus = os.cpu_count()
    if quota.exists():
        limit, period = quota.read_text().split()
        if limit != "max":
            cpus = min(cpus, int(limit) / int(period))
    maximum = Path("/sys/fs/cgroup/memory.max")
    if maximum.exists() and maximum.read_text().strip() != "max":
        memory = min(memory, int(maximum.read_text()))
    return {
        "os": platform.system(),
        "architecture": platform.machine(),
        "logical_cpus": os.cpu_count(),
        "effective_cpus": cpus,
        "memory_bytes": memory,
        "generators_share_host": True,
    }


class Lab:
    def __init__(self, args):
        self.args = args
        self.directory = Path(args.run_dir).resolve()
        self.directory.mkdir(parents=True, exist_ok=False, mode=0o700)
        self.private_dir = self.directory / "private"
        self.private_dir.mkdir(mode=0o700)
        self.results = self.directory / "results"
        self.results.mkdir()
        self.processes = {}
        self.infra_started = False
        self.native_pids = {}
        self.env = clean_environment(os.environ)
        bypass = ",".join(
            filter(
                None, [self.env.get("NO_PROXY", ""), "127.0.0.1", "localhost", "::1"]
            )
        )
        self.env["NO_PROXY"] = self.env["no_proxy"] = bypass
        self.prefix = str(args.port_prefix)
        self.http = int(self.prefix + "01")
        self.grpc = int(self.prefix + "02")
        self.proxy = int(self.prefix + "00")
        self.bff = int(self.prefix + "05")
        self.idp = int(self.prefix + "91")
        self.db_port = int(self.prefix + "32")
        self.dbname = "capacity_" + uuid.uuid4().hex
        self.chat_db = "capacity_chat_" + uuid.uuid4().hex
        self.dburl = (
            f"postgres://everruns:everruns@127.0.0.1:{self.db_port}/{self.dbname}"
        )
        self.chat_dburl = (
            f"postgres://everruns:everruns@127.0.0.1:{self.db_port}/{self.chat_db}"
        )
        self.api = f"http://127.0.0.1:{self.http}/api"
        self.edge = f"https://127.0.0.1:{self.proxy}"
        self.issuer = f"https://localhost:{self.idp}"
        self.password = secrets.token_urlsafe(32)
        self.worker_token = secrets.token_urlsafe(32)
        self.chat_key = secrets.token_hex(32)
        self.fixture_secret = secrets.token_urlsafe(32)
        self.cert_path = self.private_dir / "tls.pem"
        self.cert_key_path = self.private_dir / "tls-key.pem"
        self.identity_public = self.private_dir / "idp-public.json"
        self.users_file = self.private_dir / "console-users.json"

    def psql(self, sql, database=None):
        env = {
            **self.env,
            "PGHOST": "127.0.0.1",
            "PGPORT": str(self.db_port),
            "PGUSER": "everruns",
            "PGPASSWORD": "everruns",
            "PGDATABASE": database or self.dbname,
        }
        result = subprocess.run(
            ["psql", "-XAt", "--set", "ON_ERROR_STOP=1"],
            input=sql,
            text=True,
            env=env,
            capture_output=True,
            timeout=120,
        )
        if result.returncode:
            raise RuntimeError("fixture database command failed")
        return result.stdout

    def start(self, name, command, env=None, cwd=ROOT):
        log = open(self.directory / (name + ".log"), "ab")
        process = subprocess.Popen(
            command, cwd=cwd, env=env or self.env, stdout=log, stderr=subprocess.STDOUT
        )
        log.close()
        self.processes[name] = process
        return process

    def wait(self, url, timeout=180, verify=True):
        deadline = time.monotonic() + timeout
        with httpx.Client(verify=verify, trust_env=False, timeout=2) as client:
            while time.monotonic() < deadline:
                if any(p.poll() is not None for p in self.processes.values()):
                    raise RuntimeError(
                        "a fixture process stopped; inspect its local log"
                    )
                try:
                    if client.get(url).is_success:
                        return
                except httpx.HTTPError:
                    pass
                time.sleep(0.25)
        raise RuntimeError("fixture readiness timed out")

    def start_infra(self):
        for suffix in ["00", "01", "02", "05", "22", "32", "79", "91"]:
            with socket.socket() as probe:
                if probe.connect_ex(("127.0.0.1", int(self.prefix + suffix))) == 0:
                    raise RuntimeError("capacity port prefix is already in use")
        self.infra_started = True
        with open(self.directory / "infra.log", "w") as log:
            subprocess.run(
                ["bash", "scripts/lib/infra.sh", "start"],
                cwd=ROOT,
                env={**self.env, "PORT_PREFIX": self.prefix},
                stdout=log,
                stderr=subprocess.STDOUT,
                check=True,
            )
        self.psql(
            f"CREATE DATABASE {self.dbname}; CREATE DATABASE {self.chat_db};",
            "postgres",
        )
        directory = self.psql("SHOW data_directory;", "postgres").strip()
        self.native_pids["postgres"] = int(
            (Path(directory) / "postmaster.pid").read_text().splitlines()[0]
        )
        for role, suffix in [("nats", "22"), ("valkey", "79")]:
            found = subprocess.run(
                ["lsof", "-t", f"-iTCP:{self.prefix}{suffix}", "-sTCP:LISTEN"],
                capture_output=True,
                text=True,
            )
            if not found.stdout.strip():
                raise RuntimeError("cannot sample native fixture process: " + role)
            self.native_pids[role] = int(found.stdout.splitlines()[0])

    def start_identity(self):
        command = [
            "openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            str(self.cert_key_path),
            "-out",
            str(self.cert_path),
            "-days",
            "2",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1,DNS:*.capacity.localhost",
        ]
        subprocess.run(
            command, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        )
        config = {
            "chat_database": self.chat_dburl,
            "session_key": self.chat_key,
            "issuer": self.issuer,
            "client_id": "capacity-bench",
            "client_secret": self.fixture_secret,
            "cert": str(self.cert_path),
            "cert_key": str(self.cert_key_path),
            "idp_port": self.idp,
            "public_file": str(self.identity_public),
        }
        config_path = self.private_dir / "identity-config.json"
        private_json(config_path, config)
        self.start(
            "identity",
            ["node", "scripts/capacity/local_identity.mjs", str(config_path)],
        )
        self.wait(
            self.issuer + "/.well-known/openid-configuration",
            verify=str(self.cert_path),
        )

    def start_platform(self):
        env = {
            **self.env,
            "CAPACITY_ISOLATED": "1",
            "CAPACITY_IDP_PUBLIC": str(self.identity_public),
            "CAPACITY_USERS_PRIVATE": str(self.users_file),
            "CAPACITY_USERS": str(self.args.users),
            "DATABASE_URL": self.dburl,
            "HTTP_ADDR": f"127.0.0.1:{self.http}",
            "SERVER_GRPC_BIND_ADDR": f"127.0.0.1:{self.grpc}",
            "AUTH_MODE": "full",
            "AUTH_JWT_SECRET": secrets.token_hex(32),
            "AUTH_ADMIN_EMAIL": "capacity-admin@bench.invalid",
            "AUTH_ADMIN_PASSWORD": self.password,
            "AUTH_BASE_URL": self.edge + "/api",
            "PUBLIC_APP_URL": "https://console.capacity.localhost",
            "FRONTEND_URL": "https://console.capacity.localhost",
            "AUTH_JWT_ACCESS_TOKEN_LIFETIME": "14400",
            "WORKER_GRPC_AUTH_TOKEN": self.worker_token,
            "SECRETS_ENCRYPTION_KEY": "capacity-v1:"
            + __import__("base64").b64encode(secrets.token_bytes(32)).decode(),
            "DEV_MODE": "false",
            "DEPLOYMENT_GRADE": "dev",
            "OTEL_SDK_DISABLED": "true",
            "RATE_LIMIT_API_REQUESTS_PER_MINUTE": "0",
            "RATE_LIMIT_ORG_SESSION_CREATE_PER_MINUTE": "100000",
            "RATE_LIMIT_ORG_TOOL_CALLS_PER_MINUTE": "100000",
            "RATE_LIMIT_USER_ORG_CREATE_PER_HOUR": "10000",
            "SSE_PER_ORG_MAX": "1000",
            "SSE_GLOBAL_MAX": "10000",
            "NATS_URL": f"nats://127.0.0.1:{self.prefix}22",
            "VALKEY_URL": f"redis://127.0.0.1:{self.prefix}79",
            "RUST_LOG": "warn,everruns_server=info",
            "RUST_MIN_STACK": "33554432",
        }
        # Keep the same signing/encryption keys across an injected server restart.
        self.platform_env = env
        self.start("server", [str(self.args.fixture_bin)], env)
        self.wait(f"http://127.0.0.1:{self.http}/health")
        deadline = time.monotonic() + 180
        while not self.users_file.exists():
            if time.monotonic() > deadline:
                raise RuntimeError("identity fixture did not finish")
            time.sleep(0.25)
        worker_env = {
            **env,
            "SERVER_GRPC_ADDRESS": f"127.0.0.1:{self.grpc}",
            "MAX_CONCURRENT_TASKS": str(self.args.worker_slots),
            "MAX_CONCURRENT_MODEL_WAITS": str(self.args.model_waits),
        }
        self.start("worker", [str(self.args.worker_bin)], worker_env)

    def start_host_and_proxy(self):
        env = {
            **self.env,
            "CHAT_DOMAIN": "capacity.localhost",
            "CHAT_OIDC_ISSUER": self.issuer,
            "CHAT_OIDC_CLIENT_ID": "capacity-bench",
            "CHAT_OIDC_CLIENT_SECRET": self.fixture_secret,
            "CHAT_SESSION_KEY": self.chat_key,
            "CHAT_DATABASE_URL": self.chat_dburl,
            "CHAT_API_URL": self.api,
            "NODE_EXTRA_CA_CERTS": str(self.cert_path),
        }
        self.start(
            "work-chat",
            [
                "node",
                "node_modules/next/dist/bin/next",
                "start",
                "--port",
                str(self.bff),
                "--hostname",
                "127.0.0.1",
            ],
            env,
            ROOT / "apps/chat-ui",
        )
        self.wait(f"http://127.0.0.1:{self.bff}/health")
        # The proxy receives file paths, never the certificate/key contents.
        config = f"""{{
 admin off
 auto_https off
}}
https://:{self.proxy} {{
 bind 127.0.0.1
 tls {self.cert_path} {self.cert_key_path}
 @chat {{
  host *.capacity.localhost
  not host console.capacity.localhost
 }}
 handle @chat {{
  reverse_proxy 127.0.0.1:{self.bff}
 }}
 handle {{
  reverse_proxy 127.0.0.1:{self.http}
 }}
}}
"""
        path = self.directory / "Caddyfile"
        with open(
            os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w"
        ) as file:
            file.write(config)
        self.start(
            "proxy", ["caddy", "run", "--config", str(path), "--adapter", "caddyfile"]
        )
        self.wait(self.edge + "/health", verify=str(self.cert_path))

    def restart_server(self):
        process = self.processes["server"]
        # Crash this owned disposable fixture: graceful HTTP shutdown can wait
        # for established streams, which would test draining instead of recovery.
        process.kill()
        process.wait(timeout=15)
        self.start("server", [str(self.args.fixture_bin)], self.platform_env)
        self.wait(f"http://127.0.0.1:{self.http}/health")

    def identity_metrics(self):
        with httpx.Client(
            verify=str(self.cert_path), trust_env=False, timeout=10
        ) as client:
            return (
                client.post(
                    self.issuer + "/fixture-metrics",
                    headers={"X-Capacity-Secret": self.fixture_secret},
                    json={},
                )
                .raise_for_status()
                .json()
            )

    def stop(self):
        for process in reversed(list(self.processes.values())):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        if not self.infra_started:
            return
        subprocess.run(
            ["bash", "scripts/lib/infra.sh", "stop"],
            cwd=ROOT,
            env={**self.env, "PORT_PREFIX": self.prefix},
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
