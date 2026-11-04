#!/usr/bin/env python3
"""Local stress bench: what one agent turn costs the platform, and which settings change it.

Runs the deployment's process shape locally from release binaries (N API
servers, M gRPC workers, its own PostgreSQL, NATS and Valkey), drives the
hosted load test's agent mix (llmsim scenarios timed like GPT-6 Luna, GPT-6.1
Sol and Muse, plus 25 s shell commands) with one driver process per server, and
reports per turn: database commits, WAL bytes and syncs, statements, CPU of
PostgreSQL, servers and workers, platform overhead per model call and pickup.

Why local: those costs are the same on a laptop and in production, measurable
in minutes, and need no deploy. What only production has (network storage,
node packing, load balancers) the bench can approximate: `--cpus` pins each
role to cores, and `--fsync-delay-ms` makes every WAL flush wait like a
network disk does. Production stays the final check, not the place to search.

  python3 scripts/stress/run.py --server-bin target/release/everruns-server \
      --worker-bin target/release/everruns-worker --sessions 200
  # sweep: every combination runs on a fresh database
  ... --sweep env:MAX_CONCURRENT_MODEL_WAITS=200,2000 --sweep pg:commit_delay=0,200

See knowledge/operations/load-testing.md ("Local stress bench").
"""

from __future__ import annotations

import argparse
import base64
import importlib.util
import itertools
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(1, os.path.join(HERE, "..", "chaos"))

import infra  # noqa: E402
import metrics  # noqa: E402
from api import Api  # noqa: E402
from topology import Topology  # noqa: E402

REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
# The 2026-10-30 hosted load-test mix, so local and hosted numbers line up.
DEFAULT_MIX = "luna-agent=3,sol-agent=2,muse-agent=2,long-tool=1"
# An assumed network-disk flush for --prod-shaped. Not measured from RDS: tune
# it until the bench's WALWrite/WalSync share matches Performance Insights.
PROD_SHAPED_FSYNC_MS = 1.0


def chaos_fixtures():
    spec = importlib.util.spec_from_file_location("chaos_run", os.path.join(HERE, "..", "chaos", "run.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.setup_fixtures


def parse_args(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--server-bin", required=True)
    p.add_argument("--worker-bin", required=True)
    p.add_argument("--servers", type=int, default=2)
    p.add_argument("--workers", type=int, default=2)
    p.add_argument("--worker-slots", type=int, default=20, help="MAX_CONCURRENT_TASKS per worker (prod: 20)")
    p.add_argument("--sessions", type=int, default=100)
    p.add_argument("--messages", type=int, default=3)
    p.add_argument("--think", type=float, default=5.0)
    p.add_argument("--mix", default=DEFAULT_MIX)
    p.add_argument("--speed", type=float, default=1.0, help="scale scenario timing; 0.5 runs twice as fast")
    p.add_argument("--turn-timeout", type=float, default=300.0)
    p.add_argument("--cpus", default="", help="role=cores,... e.g. postgres=0,server=1-2,worker=3 (taskset lists)")
    p.add_argument("--fsync-delay-ms", type=float, default=0.0, help="added to every PostgreSQL fsync/fdatasync")
    p.add_argument("--prod-shaped", action="store_true",
                   help="pin roles to cores (1/4 PostgreSQL, 1/2 servers, 1/4 workers) and add a 1 ms flush delay")
    p.add_argument("--env", action="append", default=[], help="KEY=VALUE for every server and worker")
    p.add_argument("--pg", action="append", default=[], help="PostgreSQL setting KEY=VALUE")
    p.add_argument("--sweep", action="append", default=[],
                   help="env:KEY=a,b | pg:KEY=a,b | arg:NAME=a,b; repeat for a grid")
    p.add_argument("--idle-seconds", type=float, default=20.0,
                   help="measure background rates this long before the load, and subtract them")
    p.add_argument("--port-prefix", default=os.environ.get("PORT_PREFIX", "434"))
    p.add_argument("--run-dir")
    p.add_argument("--rust-log", default="warn,everruns=info")
    args = p.parse_args(argv)
    if args.prod_shaped:
        args.fsync_delay_ms = args.fsync_delay_ms or PROD_SHAPED_FSYNC_MS
        args.cpus = args.cpus or prod_shaped_cpus(os.cpu_count() or 4)
    return args


def prod_shaped_cpus(n: int) -> str:
    """One core in four for PostgreSQL, half for the servers, the rest for workers."""
    pg = max(1, n // 4)
    server = max(1, n // 2)
    last = n - 1
    srv_end = min(last, pg + server - 1)
    worker = f"{srv_end + 1}-{last}" if srv_end < last else str(last)
    return f"postgres=0-{pg - 1},server={pg}-{srv_end},worker={worker}"


def kv(items: list[str]) -> dict[str, str]:
    out = {}
    for item in items:
        key, sep, value = item.partition("=")
        if not sep:
            raise SystemExit(f"expected KEY=VALUE, got {item!r}")
        out[key.strip()] = value.strip()
    return out


def sweep_points(sweeps: list[str]) -> list[list[tuple[str, str, str]]]:
    """Cartesian product of sweep axes; each point is [(kind, key, value), ...]."""
    axes = []
    for s in sweeps:
        kind, _, rest = s.partition(":")
        key, _, values = rest.partition("=")
        if kind not in ("env", "pg", "arg") or not key or not values:
            raise SystemExit(f"bad --sweep {s!r}; use env:KEY=a,b, pg:KEY=a,b or arg:NAME=a,b")
        axes.append([(kind, key, v) for v in values.split(",")])
    return [list(point) for point in itertools.product(*axes)] if axes else [[]]


def label(point) -> str:
    return " ".join(f"{k}={v}" for _, k, v in point) or "baseline"


def apply_point(args, point):
    a = argparse.Namespace(**vars(args))
    env, pg = kv(args.env), kv(args.pg)
    for kind, key, value in point:
        if kind == "env":
            env[key] = value
        elif kind == "pg":
            pg[key] = value
        else:
            name = key.replace("-", "_")
            current = getattr(a, name)
            setattr(a, name, type(current)(value) if current is not None else value)
    return a, env, pg


def configure(topo: Topology, env: dict[str, str], cpus: dict[str, str]) -> None:
    for p in topo.servers + topo.workers:
        p.env.update(env)
        p.cmd = infra.taskset(cpus.get(p.kind), p.cmd)


def drive(args, base_urls: list[str], model: str, out_dir: str) -> list[dict]:
    """One load-test.py process per server, so the driver is never the bottleneck."""
    n = len(base_urls)
    share = [args.sessions // n + (1 if i < args.sessions % n else 0) for i in range(n)]
    procs = []
    for i, (url, sessions) in enumerate(zip(base_urls, share)):
        if not sessions:
            continue
        log = open(os.path.join(out_dir, f"driver-{i}.json"), "w")
        procs.append((subprocess.Popen(
            [sys.executable, os.path.join(REPO, "scripts", "load-test.py"), "--api", f"{url}/api",
             "--model", model, "--sessions", str(sessions), "--messages", str(args.messages),
             "--mix", args.mix, "--think", str(args.think), "--speed", str(args.speed),
             "--turn-timeout", str(args.turn_timeout), "--seed", f"d{i}"],
            stdout=log, stderr=open(os.path.join(out_dir, f"driver-{i}.err"), "w"),
            env={**os.environ, "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost"},
        ), log))
    results = []
    for proc, log in procs:
        proc.wait()
        log.close()
        try:
            with open(log.name) as f:
                results.append(json.loads(f.read()))
        except ValueError:
            results.append({"errors": 1, "error_sample": [f"driver output unreadable, see {log.name}"]})
    return results


def run_point(args, point, pg: infra.Postgres, valkey_port: int, nats_port: int, out_dir: str) -> dict:
    a, env, pg_settings = apply_point(args, point)
    os.makedirs(out_dir, exist_ok=True)
    pg.start(pg_settings, a.fsync_delay_ms)
    if shutil.which("valkey-cli"):
        subprocess.run(["valkey-cli", "-p", str(valkey_port), "FLUSHALL"], capture_output=True)
    a.database_url = pg.url
    a.nats_url = f"nats://127.0.0.1:{nats_port}"
    a.valkey_url = f"redis://127.0.0.1:{valkey_port}"
    a.secrets_key = "kek-local-v1:" + base64.b64encode(b"everruns-stress-bench-local-key!"[:32]).decode()
    cpus = kv(a.cpus.split(",")) if a.cpus else {}
    topo = Topology(a, out_dir)
    configure(topo, env, cpus)
    try:
        topo.start()
        topo.require_shared_event_delivery()
        model_ids, _ = chaos_fixtures()(Api(topo.base_urls), a)
        procs = {"server": [s.popen.pid for s in topo.servers], "worker": [w.popen.pid for w in topo.workers]}
        # One session and turn before measuring: the first session on a fresh
        # database creates the user's principals, and hundreds racing to do
        # that queue on one row lock, a cost production never pays per turn.
        warm = argparse.Namespace(**{**vars(a), "sessions": 1, "messages": 1, "think": 0.0})
        os.makedirs(os.path.join(out_dir, "warm-up"), exist_ok=True)
        drive(warm, topo.base_urls[:1], model_ids["instant"], os.path.join(out_dir, "warm-up"))
        # Idle first: background work of every process (leases, sweeps) is
        # charged per second, so per-turn numbers do not shrink just because
        # a bigger run spreads the same background over more turns.
        idle = metrics.counters(pg, procs)
        time.sleep(a.idle_seconds)
        before = metrics.counters(pg, procs)
        rate = metrics.background(idle, before, a.idle_seconds)
        # Reset so the top statements cover only the load, then re-read the
        # counters the reset zeroed.
        pg.psql("stress", "SELECT pg_stat_statements_reset()")
        before = metrics.counters(pg, procs)
        sampler = metrics.Sampler(pg, procs)
        sampler.start()
        t0, load_start_ms = time.monotonic(), time.time() * 1000
        drivers = drive(a, topo.base_urls, model_ids["instant"], out_dir)
        elapsed = time.monotonic() - t0
        load = sampler.stop()
        after = metrics.counters(pg, procs)
        ts = metrics.measured(metrics.turns(metrics.turn_rows(pg)), load_start_ms)
        summary = metrics.turn_summary(ts)
        result = {
            "label": label(point),
            "point": {f"{kind}:{k}": v for kind, k, v in point},
            "sessions": a.sessions, "servers": a.servers, "workers": a.workers,
            "worker_slots": a.worker_slots, "mix": a.mix, "cpus": a.cpus, "fsync_delay_ms": a.fsync_delay_ms,
            "seconds": round(elapsed, 1),
            "turns_per_s": round(summary["turns"] / max(1e-6, elapsed), 2),
            "driver_errors": sum(d.get("errors", 0) + d.get("stuck_count", 0) for d in drivers),
            "driver_error_sample": [e for d in drivers for e in d.get("error_sample", [])][:3],
            **summary,
            **metrics.per_turn(before, after, summary["turns"], rate, elapsed),
            "background_per_s": {k: round(v, 2) for k, v in rate.items()},
            "gross_per_turn": metrics.per_turn(before, after, summary["turns"]),
            "top_statements": metrics.top_statements(pg, summary["turns"]),
            **load,
        }
    finally:
        topo.stop()
        pg.stop()
    return result


COLUMNS = [
    ("label", "point"), ("turns", "turns"), ("failed_turns", "failed"), ("driver_errors", "drv err"),
    ("overhead_per_call_ms_p50", "ovh/call p50"), ("overhead_per_call_ms_p99", "p99"),
    ("pickup_ms_p50", "pickup p50"), ("pickup_ms_p99", "p99"),
    ("commits_per_turn", "xacts/turn"), ("wal_syncs_per_turn", "WAL syncs/turn"), ("wal_kb_per_turn", "WAL KB/turn"),
    ("statements_per_turn", "stmts/turn"), ("cpu_postgres_ms_per_turn", "pg CPU ms"),
    ("cpu_server_ms_per_turn", "server CPU ms"), ("cpu_worker_ms_per_turn", "worker CPU ms"),
    ("db_load_avg", "db load avg"), ("db_load_peak", "peak"),
]


def table(results: list[dict]) -> str:
    lines = ["| " + " | ".join(h for _, h in COLUMNS) + " |", "|" + "---|" * len(COLUMNS)]
    for r in results:
        lines.append("| " + " | ".join(str(r.get(k, "")) for k, _ in COLUMNS) + " |")
    return "\n".join(lines)


def main(argv=None) -> int:
    args = parse_args(argv)
    for binary in (args.server_bin, args.worker_bin):
        if not os.access(binary, os.X_OK):
            raise SystemExit(f"{binary} is not built; cargo build --release -p everruns-server -p everruns-worker")
    run_dir = os.path.abspath(args.run_dir or os.path.join(REPO, "target", "stress", time.strftime("%Y%m%d-%H%M%S")))
    os.makedirs(run_dir, exist_ok=True)
    # PostgreSQL's user must reach its data directory, which a run dir under
    # /root would not allow, so infrastructure lives in a world-readable root.
    infra_root = f"/tmp/everruns-stress-{args.port_prefix}"
    os.makedirs(infra_root, mode=0o755, exist_ok=True)
    os.chmod(infra_root, 0o755)
    prefix = args.port_prefix
    nats_port, valkey_port = int(f"{prefix}22"), int(f"{prefix}79")
    cpus = kv(args.cpus.split(",")) if args.cpus else {}
    pg = infra.Postgres(infra_root, int(f"{prefix}32"), cpus.get("postgres"))
    pg.init()
    daemons = [infra.nats(infra_root, nats_port), infra.valkey(infra_root, valkey_port)]
    for d in daemons:
        d.start()
    results = []
    try:
        points = sweep_points(args.sweep)
        for i, point in enumerate(points):
            print(f"[stress] point {i + 1}/{len(points)}: {label(point)}", flush=True)
            r = run_point(args, point, pg, valkey_port, nats_port, os.path.join(run_dir, f"point-{i + 1}"))
            results.append(r)
            with open(os.path.join(run_dir, "results.jsonl"), "a") as f:
                f.write(json.dumps(r) + "\n")
            print(json.dumps(r), flush=True)
    finally:
        pg.stop()
        for d in daemons:
            d.stop()
    md = table(results)
    with open(os.path.join(run_dir, "results.md"), "w") as f:
        f.write(md + "\n")
    print(md)
    print(f"[stress] results in {run_dir}")
    return 0 if all(r["failed_turns"] == 0 and r["driver_errors"] == 0 for r in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
