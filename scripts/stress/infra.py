"""Bench-owned PostgreSQL, NATS and Valkey for the stress bench.

The bench starts its own infrastructure instead of reusing the dev stack's, so
it can restart PostgreSQL between sweep points with different settings, pin it
to CPUs, and slow its flushes with the fsync shim. Data lives under the run's
infra directory and is thrown away: every sweep point gets a fresh database.

PostgreSQL refuses to run as root, so on a root sandbox it runs as the
`postgres` user (created by the distro package), like scripts/lib/infra.sh.
"""

from __future__ import annotations

import glob
import os
import shlex
import shutil
import subprocess
import time

HERE = os.path.dirname(os.path.abspath(__file__))
DB_NAME = "stress"


def pg_bin() -> str:
    found = shutil.which("pg_ctl")
    if found:
        return os.path.dirname(found)
    versions = sorted(glob.glob("/usr/lib/postgresql/*/bin/pg_ctl"))
    if not versions:
        raise RuntimeError("pg_ctl not found; install PostgreSQL (apt-get install postgresql postgresql-contrib)")
    return os.path.dirname(versions[-1])


def taskset(cores: str | None, cmd: list[str]) -> list[str]:
    return ["taskset", "-c", cores, *cmd] if cores else cmd


class Postgres:
    def __init__(self, root: str, port: int, cores: str | None):
        self.dir = os.path.join(root, "pg")
        self.data = os.path.join(self.dir, "data")
        self.log = os.path.join(self.dir, "postgres.log")
        self.port = port
        self.cores = cores
        self.bin = pg_bin()
        self.as_postgres = os.geteuid() == 0
        self.shim = None

    @property
    def url(self) -> str:
        return f"postgres://postgres@127.0.0.1:{self.port}/{DB_NAME}"

    def _run(self, cmd: list[str], env: dict[str, str] | None = None, check: bool = True):
        if self.as_postgres:
            exports = " ".join(f"{k}={shlex.quote(v)}" for k, v in (env or {}).items())
            cmd = ["su", "-s", "/bin/bash", "postgres", "-c", f"{exports} {shlex.join(cmd)}".strip()]
            env = None
        return subprocess.run(cmd, env={**os.environ, **(env or {})}, check=check,
                              capture_output=True, text=True)

    def init(self) -> None:
        os.makedirs(self.dir, exist_ok=True)
        if self.as_postgres:
            shutil.chown(self.dir, "postgres", "postgres")
        if not os.path.exists(os.path.join(self.data, "PG_VERSION")):
            self._run([f"{self.bin}/initdb", "-D", self.data, "--auth=trust", "-U", "postgres"])
        # The shim must be readable by the postgres user, so build it here.
        self.shim = os.path.join(self.dir, "fsync_delay.so")
        subprocess.run(["cc", "-shared", "-fPIC", "-O2", "-o", self.shim,
                        os.path.join(HERE, "fsync_delay.c"), "-ldl"], check=True)
        os.chmod(self.shim, 0o755)

    def start(self, settings: dict[str, str], fsync_delay_ms: float) -> None:
        conf = {
            "listen_addresses": "'127.0.0.1'",
            "port": str(self.port),
            "max_connections": "500",
            "shared_buffers": "'1GB'",
            "shared_preload_libraries": "'pg_stat_statements'",
            "pg_stat_statements.track": "all",
            "pg_stat_statements.max": "10000",
            "track_io_timing": "on",
            "track_wal_io_timing": "on",
            # The fsync shim intercepts fdatasync; open_datasync would bypass it.
            "wal_sync_method": "fdatasync",
            **settings,
        }
        with open(os.path.join(self.data, "postgresql.auto.conf"), "w") as f:
            f.writelines(f"{k} = {v}\n" for k, v in conf.items())
        env = {}
        if fsync_delay_ms > 0:
            env = {"LD_PRELOAD": self.shim, "FSYNC_DELAY_US": str(int(fsync_delay_ms * 1000))}
        self._run(taskset(self.cores, [f"{self.bin}/pg_ctl", "-D", self.data, "-l", self.log, "-w", "start"]), env)
        self.psql("postgres", f"DROP DATABASE IF EXISTS {DB_NAME} WITH (FORCE)")
        self.psql("postgres", f"CREATE DATABASE {DB_NAME}")
        self.psql(DB_NAME, "CREATE EXTENSION IF NOT EXISTS pg_stat_statements")

    def stop(self) -> None:
        if os.path.exists(os.path.join(self.data, "postmaster.pid")):
            self._run([f"{self.bin}/pg_ctl", "-D", self.data, "-m", "fast", "-w", "stop"], check=False)

    def postmaster_pid(self) -> int:
        with open(os.path.join(self.data, "postmaster.pid")) as f:
            return int(f.readline())

    def psql(self, db: str, sql: str) -> list[list[str]]:
        out = subprocess.run(
            [f"{self.bin}/psql", "-h", "127.0.0.1", "-p", str(self.port), "-U", "postgres", "-d", db,
             "-X", "-q", "-At", "-F", "\t", "-v", "ON_ERROR_STOP=1", "-c", sql],
            check=True, capture_output=True, text=True,
        ).stdout
        return [line.split("\t") for line in out.splitlines() if line]


class Daemon:
    """NATS or Valkey: one process, stopped with the run."""

    def __init__(self, name: str, cmd: list[str], log: str):
        self.name, self.cmd, self.log = name, cmd, log
        self.popen: subprocess.Popen | None = None

    def start(self) -> None:
        self.popen = subprocess.Popen(self.cmd, stdout=open(self.log, "ab"), stderr=subprocess.STDOUT,
                                      stdin=subprocess.DEVNULL, start_new_session=True)
        time.sleep(1)
        if self.popen.poll() is not None:
            raise RuntimeError(f"{self.name} exited at startup, see {self.log}")

    def stop(self) -> None:
        if self.popen and self.popen.poll() is None:
            self.popen.terminate()
            self.popen.wait(timeout=10)


def nats(root: str, port: int) -> Daemon:
    binary = shutil.which("nats-server")
    if not binary:
        raise RuntimeError("nats-server not found: two servers need JetStream for shared event delivery")
    store = os.path.join(root, "nats")
    shutil.rmtree(store, ignore_errors=True)
    return Daemon("nats", [binary, "-js", "-a", "127.0.0.1", "-p", str(port), "-sd", store],
                  os.path.join(root, "nats.log"))


def valkey(root: str, port: int) -> Daemon:
    binary = shutil.which("valkey-server") or shutil.which("redis-server")
    if not binary:
        raise RuntimeError("valkey-server not found (apt-get install valkey-server)")
    return Daemon("valkey", [binary, "--bind", "127.0.0.1", "--port", str(port), "--save", "",
                             "--appendonly", "no", "--dir", root], os.path.join(root, "valkey.log"))
