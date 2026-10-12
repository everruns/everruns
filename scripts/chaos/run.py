#!/usr/bin/env python3
"""Multi-process chaos and load harness for Everruns.

Starts 2 API servers and 3 workers (release binaries) against an already
running PostgreSQL / NATS / Valkey, drives N sessions of mixed turn shapes over
the public HTTP API, SIGKILLs and restarts a worker and a server on a schedule,
then asserts turn-level correctness and prints metrics. Exit code 0 = PASS.

  PORT_PREFIX=433 bash scripts/lib/infra.sh start     # infra, once
  python3 scripts/chaos/run.py \
      --server-bin target/release/everruns-server \
      --worker-bin target/release/everruns-worker --sessions 200 [--no-chaos]

See knowledge/operations/load-testing.md ("Chaos harness").
"""

from __future__ import annotations

import argparse
import base64
import concurrent.futures as cf
import json
import os
import random
import re
import subprocess
import sys
import threading
import time
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from api import Api  # noqa: E402
from check import (  # noqa: E402
    ActorLeaseSampler,
    Report,
    actor_releases,
    actor_worker_overlap,
    check_session,
    pct,
    worker_overlap,
)
from load import Driver, plan_kinds  # noqa: E402
from topology import Topology  # noqa: E402

DEFAULT_CHAOS = "kill:worker-0@15,restart:worker-0@21,kill:server-1@35,restart:server-1@41"


def parse_args():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--server-bin", required=True)
    p.add_argument("--worker-bin", required=True)
    p.add_argument("--sessions", type=int, default=200)
    p.add_argument("--concurrency", type=int, default=50, help="sessions driven at once")
    p.add_argument("--servers", type=int, default=2)
    p.add_argument("--workers", type=int, default=3)
    p.add_argument("--worker-slots", type=int, default=100, help="MAX_CONCURRENT_TASKS per worker")
    p.add_argument("--port-prefix", default=os.environ.get("PORT_PREFIX", "433"))
    p.add_argument("--database-url")
    p.add_argument("--nats-url")
    p.add_argument("--valkey-url")
    # Fixed throwaway key: the chaos DB holds only llmsim test data.
    p.add_argument("--secrets-key", default="kek-local-v1:" + base64.b64encode(b"everruns-chaos-harness-local-key"[:32]).decode())
    p.add_argument("--rust-log", default="info,everruns_worker::actor_runtime=debug")
    p.add_argument("--chaos", default=DEFAULT_CHAOS, help="comma list of kill|restart:<proc>@<seconds>")
    p.add_argument("--no-chaos", action="store_true")
    p.add_argument("--only", choices=["plain", "tools", "midturn", "cancel", "approval"])
    p.add_argument("--plain-turns", type=int, default=3)
    p.add_argument("--turn-timeout", type=float, default=120)
    p.add_argument("--quiesce-timeout", type=float, default=180)
    p.add_argument("--run-dir")
    p.add_argument("--seed", type=int, default=7)
    p.add_argument("--keep-running", action="store_true", help="leave the topology up (debugging)")
    a = p.parse_args()
    pre = a.port_prefix
    a.database_url = a.database_url or f"postgres://everruns:everruns@localhost:{pre}32/everruns"
    a.nats_url = a.nats_url or f"nats://localhost:{pre}22"
    a.valkey_url = a.valkey_url or f"redis://localhost:{pre}79"
    a.server_bin = os.path.abspath(a.server_bin)
    a.worker_bin = os.path.abspath(a.worker_bin)
    return a


def psql(db_url: str, sql: str) -> str | None:
    try:
        out = subprocess.run(
            ["psql", db_url, "-At", "-c", sql], capture_output=True, text=True, timeout=30
        )
        return out.stdout.strip() if out.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def setup_fixtures(api: Api, args) -> tuple[dict, dict]:
    """Harness-owned llmsim models plus three agents.

    The seeded llmsim models start disabled, and every server start re-applies
    the seed, which resets `enabled` back to false. A chaos run restarts a
    server, so enabling the seeded models would make session creation fail
    with "Model not found" after the restart. The harness therefore creates
    its own models under the seeded llmsim provider, which the seed does not
    touch. llmsim derives behavior from the name: `-realistic` anywhere in it
    turns on the agent-shaped workload with default (0.9 s) timing.
    """
    models = api.get("/v1/models", query={"limit": 500})
    items = models.get("data", models) if isinstance(models, dict) else models
    by_name = {}
    provider_id = None
    for m in items:
        name = m.get("model_id") or m.get("name")
        by_name[name] = m
        if name and name.startswith("llmsim") and m.get("provider_id"):
            provider_id = provider_id or m.get("provider_id")
    if not provider_id:
        raise RuntimeError(f"no llmsim provider; models: {sorted(by_name)}")
    want = {"instant": "llmsim-chaos", "realistic": "llmsim-chaos-realistic"}
    model_ids = {}
    for key, name in want.items():
        m = by_name.get(name)
        if m is None:
            m = api.post(
                f"/v1/providers/{provider_id}/models",
                {
                    "model_id": name,
                    "display_name": name,
                    "capabilities": ["chat", "tools"],
                    "enabled": True,
                },
            )
        elif not m.get("enabled", True):
            api.request("PATCH", f"/v1/models/{m['id']}", body={"enabled": True})
        model_ids[key] = m["id"]

    tag = time.strftime("%H%M%S") + "-" + os.urandom(2).hex()
    agents = {}
    specs = {
        "plain": [],
        "tools": [{"ref": "bashkit_shell"}],
        "approval": [{"ref": "bashkit_shell"}, {"ref": "tool_approval", "config": {"mode": "protective"}}],
    }
    for key, caps in specs.items():
        r = api.post(
            "/v1/agents",
            {
                "name": f"chaos-{key}-{tag}",
                "system_prompt": "You are a load-test agent.",
                "default_model_id": model_ids["instant"],
                "capabilities": caps,
            },
        )
        agents[key] = r["id"]
    return model_ids, agents


def chaos_thread(topo: Topology, schedule: list, t0: float, log: list, stop: threading.Event):
    for action, name, at in schedule:
        while time.time() < t0 + at:
            if stop.wait(0.05):
                return
        p = topo.find(name)
        if action == "kill":
            t = p.sigkill()
            log.append({"action": "kill", "proc": name, "t": t})
            print(f"[chaos] +{t - t0:5.1f}s SIGKILL {name}", flush=True)
        else:
            p.start()
            ok = p.wait_healthy(60)
            log.append({"action": "restart", "proc": name, "t": time.time(), "healthy": ok})
            print(f"[chaos] +{time.time() - t0:5.1f}s restarted {name} healthy={ok}", flush=True)


def parse_schedule(s: str):
    out = []
    for item in filter(None, (x.strip() for x in s.split(","))):
        action, rest = item.split(":", 1)
        name, at = rest.split("@", 1)
        assert action in ("kill", "restart"), item
        out.append((action, name, float(at)))
    return sorted(out, key=lambda x: x[2])


def main():
    args = parse_args()
    random.seed(args.seed)
    run_dir = args.run_dir or os.path.join(
        os.environ.get("TMPDIR", "/tmp"), "everruns-chaos", time.strftime("%Y%m%d-%H%M%S")
    )
    os.makedirs(run_dir, exist_ok=True)
    print(f"run dir: {run_dir}", flush=True)

    topo = Topology(args, run_dir)
    rep = Report()
    timings = defaultdict(list)
    chaos_log: list = []
    exit_code = 1
    try:
        topo.start()
        print(f"topology up: servers {topo.base_urls}, {len(topo.workers)} workers", flush=True)
        # Every worker must have reached its server (registered over gRPC).
        # A port clash once had two workers talking to a sandbox proxy while
        # the third did all the work, which makes a worker kill meaningless.
        want_ids = {w.env["WORKER_ID"] for w in topo.workers}
        for _ in range(60):
            got = psql(
                args.database_url,
                "select id from durable_workers where last_heartbeat_at > now() - interval '30 seconds'",
            )
            if got is not None and want_ids <= set(got.split()):
                break
            time.sleep(1)
        else:
            raise RuntimeError(f"workers not registered: {sorted(want_ids - set((got or '').split()))}")
        api = Api(topo.base_urls)
        model_ids, agents = setup_fixtures(api, args)
        driver = Driver(api, args, model_ids, agents)

        have_pss = psql(args.database_url, "select pg_stat_statements_reset()") is not None
        sampler = None
        if ActorLeaseSampler.available(args.database_url):
            sampler = ActorLeaseSampler(args.database_url)
            threading.Thread(target=sampler.run, daemon=True).start()
            print("[check] session_actors present: sampling actor leases", flush=True)
        kinds = plan_kinds(args.sessions, args.only)
        stop = threading.Event()
        t0 = time.time()
        ct = None
        if not args.no_chaos:
            ct = threading.Thread(
                target=chaos_thread,
                args=(topo, parse_schedule(args.chaos), t0, chaos_log, stop),
                daemon=True,
            )
            ct.start()
        with cf.ThreadPoolExecutor(max_workers=args.concurrency) as pool:
            futs = []
            for i, k in enumerate(kinds):
                futs.append(pool.submit(driver.run_session, i, k))
                time.sleep(0.02)  # gentle ramp for session creation
            done = 0
            for f in cf.as_completed(futs):
                done += 1
                if done % 25 == 0:
                    print(f"[load] +{time.time() - t0:5.1f}s {done}/{len(futs)} sessions done", flush=True)
        t_load_end = time.time()
        if ct:
            ct.join()  # finish the schedule (restarts) even if load ended early
        stop.set()
        for p in topo.servers + topo.workers:
            if not p.alive():
                print(f"[chaos] {p.name} is down at end of load; restarting", flush=True)
                p.start()
                p.wait_healthy(60)

        # Quiescence: every session idle, or time out.
        print("[check] waiting for quiescence", flush=True)
        deadline = time.time() + args.quiesce_timeout
        pending = [r for r in driver.runs if r.session_id]
        sessions = {}
        while pending and time.time() < deadline:
            nxt = []
            for r in pending:
                try:
                    s = api.get(f"/v1/sessions/{r.session_id}")
                except Exception:  # noqa: BLE001
                    nxt.append(r)
                    continue
                sessions[r.session_id] = s
                if s.get("status") != "idle":
                    nxt.append(r)
            pending = nxt
            if pending:
                time.sleep(2)
        t_quiet = time.time()
        rep.counts["sessions_not_quiet_at_timeout"] = len(pending)

        # Assertions over persisted events.
        print("[check] reading events", flush=True)
        all_events = {}
        for r in driver.runs:
            if not r.session_id:
                rep.fail("session_create_failed", f"#{r.index}", "; ".join(r.errors)[:200])
                continue
            evs = api.events(r.session_id)
            all_events[r.session_id] = evs
            try:
                sessions[r.session_id] = api.get(f"/v1/sessions/{r.session_id}")
            except Exception:  # noqa: BLE001
                pass
            check_session(r, evs, sessions.get(r.session_id), rep, timings)
        if sampler:
            sampler.stop = True
        # Actor checks only when the build actually leased sessions: main
        # also has an (unused, empty) session_actors table.
        if sampler and sampler.history:
            sampler.check(rep, actor_releases(run_dir, args.workers))
            with open(os.path.join(run_dir, "actor_history.json"), "w") as f:
                json.dump(sampler.history, f)
            actor_worker_overlap(run_dir, args.workers, rep)
        elif sampler:
            sampler = None
        spans = worker_overlap(run_dir, args.workers, args.database_url, chaos_log, rep)
        if not spans and not sampler:
            rep.notes.append("two-worker check: no task spans in worker logs and no session_actors table; not checked")

        # Work per worker, to show the kills hit workers that were busy.
        per_worker = {}
        for w in topo.workers:
            tasks = leased = 0
            with open(w.log_path, errors="replace") as f:
                for line in f:
                    if "Executing task" in line:
                        tasks += 1
                    elif "Leased session actors" in line:
                        mm = re.search(r"leased=(\d+)", line)
                        leased += int(mm.group(1)) if mm else 1
            per_worker[w.name] = {"tasks_executed": tasks, "actors_leased": leased}

        # Metrics.
        n_turns = rep.counts.get("turn.completed", 0)
        dur = t_load_end - t0
        metrics = {
            "sessions": len(driver.runs),
            "kinds": {k: kinds.count(k) for k in sorted(set(kinds))},
            "load_seconds": round(dur, 1),
            "quiesce_seconds": round(t_quiet - t_load_end, 1),
            "turns_completed": n_turns,
            "turns_per_second": round(n_turns / dur, 2) if dur else None,
            "approvals_answered": sum(r.approvals for r in driver.runs),
            "cancels_sent": sum(len(r.cancels) for r in driver.runs),
            "api_requests": api.request_count,
            "api_conn_errors": api.conn_errors,
            "api_retries": api.retries,
            "driver_errors": sum(len(r.errors) for r in driver.runs),
            "per_worker": per_worker,
        }
        lat = {}
        for k, v in sorted(timings.items()):
            if k.startswith("e2e_ms:") or k in ("server_ms", "pickup_ms"):
                lat[k] = {
                    "n": len(v),
                    "p50": round(pct(v, 50) or 0),
                    "p95": round(pct(v, 95) or 0),
                    "p99": round(pct(v, 99) or 0),
                    "max": round(max(v)) if v else None,
                }
        metrics["latency_ms"] = lat
        # Recovery: turns open at a kill, and when the last of them ended.
        rec = []
        for c in chaos_log:
            if c["action"] != "kill":
                continue
            t = c["t"]
            hit = [sp for sp in timings["turn_spans"] if sp[0] <= t < sp[1]]
            last = max((sp[1] for sp in hit), default=None)
            rec.append(
                {
                    "kill": c["proc"],
                    "at_s": round(t - t0, 1),
                    "turns_open": len(hit),
                    "recovery_s": round(last - t, 1) if last else 0.0,
                    "ended": dict(
                        sorted(
                            {e: sum(1 for sp in hit if sp[4] == e) for e in {sp[4] for sp in hit}}.items()
                        )
                    ),
                }
            )
        metrics["recovery"] = rec
        if have_pss:
            total = psql(
                args.database_url,
                "select coalesce(sum(calls),0) from pg_stat_statements "
                "where query not ilike '%pg_stat_statements%'",
            )
            try:
                metrics["pg_statements_total"] = int(total)
                turns_all = sum(rep.counts.get(t, 0) for t in ("turn.completed", "turn.cancelled", "turn.failed"))
                metrics["pg_statements_per_turn"] = round(int(total) / max(turns_all, 1), 1)
            except (TypeError, ValueError):
                pass
        else:
            metrics["pg_statements_per_turn"] = "n/a (pg_stat_statements not loaded)"

        result = {
            "verdict": "PASS" if not rep.failures else "FAIL",
            "chaos": chaos_log,
            "metrics": metrics,
            "counts": dict(rep.counts),
            "failures": {k: v for k, v in rep.failures.items()},
            "notes": rep.notes,
            "driver_errors_sample": [
                f"{r.session_id or '#' + str(r.index)} ({r.kind}): {e}" for r in driver.runs for e in r.errors
            ][:40],
        }
        with open(os.path.join(run_dir, "result.json"), "w") as f:
            json.dump(result, f, indent=2, default=str)
        with open(os.path.join(run_dir, "runs.json"), "w") as f:
            json.dump(
                [
                    {
                        "index": r.index,
                        "kind": r.kind,
                        "session_id": r.session_id,
                        "errors": r.errors,
                        "messages": [m.__dict__ for m in r.messages],
                    }
                    for r in driver.runs
                ],
                f,
                indent=1,
                default=str,
            )
        print_summary(result, timings)
        exit_code = 0 if not rep.failures else 1
    finally:
        if not args.keep_running:
            topo.stop()
    print(f"run dir: {run_dir}")
    sys.exit(exit_code)


def print_summary(result, timings):
    m = result["metrics"]
    print("\n================ chaos harness summary ================")
    print(f"sessions {m['sessions']} {m['kinds']}  load {m['load_seconds']}s  quiesce {m['quiesce_seconds']}s")
    print(
        f"turns completed {m['turns_completed']}  ({m['turns_per_second']} turns/s)  "
        f"approvals {m['approvals_answered']}  cancels {m['cancels_sent']}"
    )
    print(f"counts: {result['counts']}")
    for k, v in m["latency_ms"].items():
        print(f"  {k:24s} n={v['n']:4d} p50={v['p50']} p95={v['p95']} p99={v['p99']} max={v['max']}")
    for r in m["recovery"]:
        print(f"  recovery after kill {r['kill']} at +{r['at_s']}s: {r['turns_open']} open turns, last ended +{r['recovery_s']}s {r['ended']}")
    print(f"  pg statements/turn: {m.get('pg_statements_per_turn')}")
    print(f"  per worker: {m.get('per_worker')}")
    print(f"  api requests {m['api_requests']} conn errors {m['api_conn_errors']} retries {m['api_retries']} driver errors {m['driver_errors']}")
    for n in result.get("notes", []):
        print(f"  note: {n}")
    if result["failures"]:
        print("\nFAIL")
        for check, items in result["failures"].items():
            print(f"  {check}: {len(items)}")
            for it in items[:10]:
                print(f"    {it}")
    else:
        print("\nPASS")
    if result["driver_errors_sample"]:
        print("driver errors (sample):")
        for e in result["driver_errors_sample"][:10]:
            print(f"  {e}")


if __name__ == "__main__":
    main()
