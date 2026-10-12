"""Process topology for the chaos harness: N API servers and M workers.

Every process is a release binary started directly (no cargo, no Doppler),
sharing one PostgreSQL, NATS and Valkey. Workers are spread round robin over
the servers' gRPC ports, so killing one server strands only its workers until
it comes back (they reconnect to the same address, as a deployment's worker
would to its load balancer). Logs go to <run_dir>/<name>.log, one per process
incarnation appended in order, with a marker line on every (re)start.
"""

from __future__ import annotations

import os
import signal
import subprocess
import time
import urllib.request
from dataclasses import dataclass, field


@dataclass
class Proc:
    name: str
    kind: str  # "server" | "worker"
    cmd: list[str]
    env: dict[str, str]
    log_path: str
    health_url: str | None = None
    popen: subprocess.Popen | None = None
    starts: list[float] = field(default_factory=list)
    kills: list[float] = field(default_factory=list)

    def start(self) -> None:
        log = open(self.log_path, "ab", buffering=0)
        log.write(f"\n===== chaos: start {self.name} at {time.time():.3f} =====\n".encode())
        self.popen = subprocess.Popen(
            self.cmd,
            env=self.env,
            stdout=log,
            stderr=subprocess.STDOUT,
            stdin=subprocess.DEVNULL,
            start_new_session=True,
        )
        log.close()
        self.starts.append(time.time())

    def alive(self) -> bool:
        return self.popen is not None and self.popen.poll() is None

    def sigkill(self) -> float:
        t = time.time()
        if self.alive():
            os.killpg(self.popen.pid, signal.SIGKILL)
            self.popen.wait()
        self.kills.append(t)
        with open(self.log_path, "ab") as log:
            log.write(f"\n===== chaos: SIGKILL {self.name} at {t:.3f} =====\n".encode())
        return t

    def stop(self) -> None:
        if not self.alive():
            return
        try:
            os.killpg(self.popen.pid, signal.SIGTERM)
            self.popen.wait(timeout=15)
        except (subprocess.TimeoutExpired, ProcessLookupError):
            try:
                os.killpg(self.popen.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            self.popen.wait()

    def wait_healthy(self, timeout: float) -> bool:
        if not self.health_url:
            return True
        deadline = time.time() + timeout
        while time.time() < deadline:
            if not self.alive():
                return False
            try:
                with urllib.request.urlopen(self.health_url, timeout=2) as r:
                    if r.status == 200:
                        return True
            except Exception:
                pass
            time.sleep(0.5)
        return False


class Topology:
    def __init__(self, args, run_dir: str):
        self.args = args
        self.run_dir = run_dir
        self.servers: list[Proc] = []
        self.workers: list[Proc] = []
        prefix = args.port_prefix
        base_env = dict(os.environ)
        # Never hand real provider keys to the stack: llmsim only.
        for k in list(base_env):
            if k.endswith("_API_KEY") or k in ("DOPPLER_TOKEN",):
                base_env.pop(k, None)
            # A sandbox HTTP(S) proxy must not see the worker's gRPC hop.
            if k.lower() in ("http_proxy", "https_proxy", "all_proxy", "grpc_proxy"):
                base_env.pop(k, None)
        base_env["NO_PROXY"] = base_env["no_proxy"] = "127.0.0.1,localhost"
        token = "chaos-" + os.urandom(12).hex()
        common = {
            "DATABASE_URL": args.database_url,
            "NATS_URL": args.nats_url,
            "VALKEY_URL": args.valkey_url,
            "AUTH_MODE": "none",
            "DEV_MODE": "false",
            "WORKER_GRPC_AUTH_TOKEN": token,
            "RUST_LOG": args.rust_log,
            "OTEL_SDK_DISABLED": "true",
            "DEPLOYMENT_GRADE": "dev",
            # The driver is one IP; the per-IP and per-org limits would
            # measure the limiter, not the platform.
            "RATE_LIMIT_API_REQUESTS_PER_MINUTE": "0",
            "RATE_LIMIT_ORG_SESSION_CREATE_PER_MINUTE": "100000",
            "RATE_LIMIT_ORG_TOOL_CALLS_PER_MINUTE": "100000",
            "SECRETS_ENCRYPTION_KEY": args.secrets_key,
            # One SSE stream per live session, all from the default org.
            "SSE_PER_ORG_MAX": "5000",
            "SSE_GLOBAL_MAX": "10000",
        }
        for i in range(args.servers):
            http_port = int(f"{prefix}{41 + i:02d}")
            # 51 collides with the sandbox agent proxy on 43351 at prefix 433.
            grpc_port = int(f"{prefix}{45 + i:02d}")
            env = dict(base_env, **common)
            env.update(
                {
                    "HTTP_ADDR": f"127.0.0.1:{http_port}",
                    "SERVER_GRPC_BIND_ADDR": f"127.0.0.1:{grpc_port}",
                    "API_BASE_URL": f"http://127.0.0.1:{http_port}",
                    "PUBLIC_APP_URL": f"http://127.0.0.1:{http_port}",
                    "APP_URL": f"http://127.0.0.1:{http_port}",
                    "CORS_ALLOWED_ORIGINS": f"http://127.0.0.1:{http_port}",
                }
            )
            p = Proc(
                name=f"server-{i}",
                kind="server",
                cmd=[args.server_bin],
                env=env,
                log_path=os.path.join(run_dir, f"server-{i}.log"),
                health_url=f"http://127.0.0.1:{http_port}/health",
            )
            p.http_port = http_port  # type: ignore[attr-defined]
            p.grpc_port = grpc_port  # type: ignore[attr-defined]
            self.servers.append(p)
        for j in range(args.workers):
            srv = self.servers[j % len(self.servers)]
            env = dict(base_env, **common)
            env.update(
                {
                    "SERVER_GRPC_ADDRESS": f"127.0.0.1:{srv.grpc_port}",  # type: ignore[attr-defined]
                    "WORKER_ID": f"chaos-worker-{j}",
                    "MAX_CONCURRENT_TASKS": str(args.worker_slots),
                }
            )
            self.workers.append(
                Proc(
                    name=f"worker-{j}",
                    kind="worker",
                    cmd=[args.worker_bin],
                    env=env,
                    log_path=os.path.join(run_dir, f"worker-{j}.log"),
                )
            )

    @property
    def base_urls(self) -> list[str]:
        return [f"http://127.0.0.1:{s.http_port}" for s in self.servers]  # type: ignore[attr-defined]

    def start(self) -> None:
        # First server alone: it runs migrations; a concurrent second
        # migrator would only measure the advisory lock.
        first, rest = self.servers[0], self.servers[1:]
        first.start()
        if not first.wait_healthy(180):
            raise RuntimeError(f"{first.name} did not become healthy, see {first.log_path}")
        for s in rest:
            s.start()
        for s in rest:
            if not s.wait_healthy(120):
                raise RuntimeError(f"{s.name} did not become healthy, see {s.log_path}")
        for w in self.workers:
            w.start()
        time.sleep(3)
        dead = [w.name for w in self.workers if not w.alive()]
        if dead:
            raise RuntimeError(f"workers exited at startup: {dead}")

    def find(self, name: str) -> Proc:
        for p in self.servers + self.workers:
            if p.name == name:
                return p
        raise KeyError(name)

    def stop(self) -> None:
        for p in self.workers + self.servers:
            p.stop()
