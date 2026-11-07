"""What the stress bench measures, per sweep point.

Totals come from counters read before and after the load (PostgreSQL's
pg_stat_database, pg_stat_wal and pg_stat_statements, and each process's CPU
time from /proc), divided by the turns the run completed. A sampler polls
pg_stat_activity twice a second for database load in the shape RDS Performance
Insights reports it: average active sessions, split by wait event. Turn timing
comes from the persisted events, the same definition the hosted load tests use:

    pickup   = turn.started - input.message
    overhead = (turn end - turn.started) - model time (llm.generation) - tool time (act.completed)
"""

from __future__ import annotations

import collections
import os
import re
import json
import threading
import time
import urllib.request

CLK_TCK = os.sysconf("SC_CLK_TCK")


def proc_cpu_seconds(pid: int, children: bool = False) -> float:
    """utime + stime of a process; with children, also of its reaped children."""
    try:
        with open(f"/proc/{pid}/stat") as f:
            fields = f.read().rsplit(")", 1)[1].split()
    except (FileNotFoundError, ProcessLookupError):
        return 0.0
    # Fields after the comm: state is [0], ppid [1], utime [11], stime [12], cutime [13], cstime [14].
    ticks = int(fields[11]) + int(fields[12])
    if children:
        ticks += int(fields[13]) + int(fields[14])
    return ticks / CLK_TCK


def child_pids(parent: int) -> list[int]:
    out = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/stat") as f:
                if int(f.read().rsplit(")", 1)[1].split()[1]) == parent:
                    out.append(int(entry))
        except (FileNotFoundError, ProcessLookupError, IndexError, ValueError):
            continue
    return out


def rss_mib(pid: int) -> float:
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024
    except (FileNotFoundError, ProcessLookupError):
        pass
    return 0.0


def nats_stream(monitor_port: int | None) -> dict[str, float]:
    """Bytes and messages held in NATS JetStream streams (the live-event replay buffer)."""
    if not monitor_port:
        return {}
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{monitor_port}/jsz?streams=true", timeout=5) as r:
            jsz = json.load(r)
    except (OSError, ValueError):
        return {}
    streams = [s for a in jsz.get("account_details", []) for s in a.get("stream_detail", [])]
    return {"nats_bytes": float(sum(s["state"]["bytes"] for s in streams)),
            "nats_messages": float(sum(s["state"]["messages"] for s in streams))}


def postgres_cpu_seconds(postmaster: int) -> float:
    # Backends that exited are in the postmaster's cutime; live ones are summed.
    return proc_cpu_seconds(postmaster, children=True) + sum(proc_cpu_seconds(p) for p in child_pids(postmaster))


def counters(pg, procs: dict[str, list[int]]) -> dict[str, float]:
    db = pg.psql("stress", "SELECT xact_commit, xact_rollback FROM pg_stat_database WHERE datname = 'stress'")[0]
    wal = pg.psql("stress", "SELECT wal_records, wal_bytes, wal_write, wal_sync, wal_sync_time FROM pg_stat_wal")[0]
    stm = pg.psql("stress", "SELECT coalesce(sum(calls), 0), coalesce(sum(total_exec_time), 0) FROM pg_stat_statements")[0]
    out = {
        "commits": float(db[0]), "rollbacks": float(db[1]),
        "wal_records": float(wal[0]), "wal_bytes": float(wal[1]), "wal_writes": float(wal[2]),
        "wal_syncs": float(wal[3]), "wal_sync_ms": float(wal[4]),
        "statements": float(stm[0]), "statement_ms": float(stm[1]),
        "cpu_postgres": postgres_cpu_seconds(pg.postmaster_pid()),
    }
    for role, pids in procs.items():
        out[f"cpu_{role}"] = sum(proc_cpu_seconds(p) for p in pids)
    return out


class Sampler(threading.Thread):
    """Database load (active backends by wait event) and CPU per role, sampled during the load."""

    def __init__(self, pg, procs: dict[str, list[int]], interval: float = 0.5):
        super().__init__(daemon=True)
        self.pg, self.procs, self.interval = pg, procs, interval
        self.stop_event = threading.Event()
        self.samples = 0
        self.waits: collections.Counter = collections.Counter()
        self.active: list[int] = []
        self.cpu: dict[str, list[float]] = collections.defaultdict(list)
        self.rss: dict[str, list[float]] = collections.defaultdict(list)

    def run(self) -> None:
        last, last_t = None, time.monotonic()
        while not self.stop_event.wait(self.interval):
            try:
                rows = self.pg.psql("stress", (
                    "SELECT coalesce(wait_event_type || ':' || wait_event, 'CPU') FROM pg_stat_activity "
                    "WHERE datname = 'stress' AND state = 'active' AND pid <> pg_backend_pid()"))
            except Exception:  # noqa: BLE001 - a missed sample is not a failed run
                continue
            self.samples += 1
            self.active.append(len(rows))
            self.waits.update(r[0] for r in rows)
            now = time.monotonic()
            cpu = {"postgres": postgres_cpu_seconds(self.pg.postmaster_pid())}
            for role, pids in self.procs.items():
                cpu[role] = sum(proc_cpu_seconds(p) for p in pids)
            for role, pids in self.procs.items():
                self.rss[role].append(sum(rss_mib(p) for p in pids))
            if last is not None:
                for role, v in cpu.items():
                    self.cpu[role].append((v - last[role]) / max(1e-6, now - last_t))
            last, last_t = cpu, now

    def stop(self) -> dict:
        self.stop_event.set()
        self.join(5)
        n = max(1, self.samples)
        return {
            "db_load_avg": round(sum(self.active) / n, 2),
            "db_load_peak": max(self.active, default=0),
            "db_waits": {k: round(v / n, 2) for k, v in self.waits.most_common(8)},
            # Peak over a 2 s moving window, so one sample's jitter does not set it.
            "cpu_peak_cores": {role: round(max(moving(v, 4), default=0), 2) for role, v in self.cpu.items()},
            # Resident memory per role, summed over its processes: at the first
            # sample (idle, after warm-up) and at the peak.
            "rss_idle_mib": {role: round(v[0]) for role, v in self.rss.items() if v},
            "rss_peak_mib": {role: round(max(v)) for role, v in self.rss.items() if v},
        }


def moving(values: list[float], width: int) -> list[float]:
    return [sum(values[i:i + width]) / width for i in range(max(0, len(values) - width + 1))]


TURN_EVENTS = ("input.message", "turn.started", "llm.generation", "act.completed",
               "turn.completed", "turn.failed", "turn.cancelled", "turn.sealed")


def turn_rows(pg) -> list[list[str]]:
    types = ", ".join(f"'{t}'" for t in TURN_EVENTS)
    return pg.psql("stress", (
        "SELECT session_id, event_type, (extract(epoch FROM ts) * 1000)::bigint, "
        "coalesce(data->'metadata'->>'duration_ms', data->>'duration_ms', '0') "
        f"FROM events WHERE event_type IN ({types}) ORDER BY session_id, sequence"))


def turns(rows: list[list[str]]) -> list[dict]:
    out, cur, inp, session = [], None, None, None
    for sid, kind, ts, dur in rows:
        if sid != session:
            cur, inp, session = None, None, sid
        t, d = float(ts), float(dur or 0)
        if kind == "input.message":
            inp = t
        elif kind == "turn.started":
            cur = {"start": t, "pickup": t - inp if inp is not None else None, "llm": 0.0, "tool": 0.0, "calls": 0}
        elif cur and kind == "llm.generation":
            cur["llm"] += d
            cur["calls"] += 1
        elif cur and kind == "act.completed":
            cur["tool"] += d
        elif cur and kind in ("turn.completed", "turn.failed", "turn.cancelled", "turn.sealed"):
            cur["turn"] = t - cur["start"]
            cur["ok"] = kind == "turn.completed"
            cur["overhead"] = cur["turn"] - cur["llm"] - cur["tool"]
            out.append(cur)
            cur, inp = None, None
    return out


def measured(ts: list[dict], since_ms: float) -> list[dict]:
    """Turns that started once the load began, leaving out the warm-up."""
    return [t for t in ts if t["start"] >= since_ms]


def pct(values, p):
    values = sorted(v for v in values if v is not None)
    if not values:
        return None
    return round(values[min(len(values) - 1, int(round(p / 100 * (len(values) - 1))))])


def turn_summary(ts: list[dict]) -> dict:
    per_call = [t["overhead"] / max(1, t["calls"]) for t in ts]
    return {
        "turns": len(ts),
        "failed_turns": sum(not t["ok"] for t in ts),
        "model_calls_per_turn": round(sum(t["calls"] for t in ts) / max(1, len(ts)), 2),
        "overhead_per_call_ms_p50": pct(per_call, 50),
        "overhead_per_call_ms_p99": pct(per_call, 99),
        "pickup_ms_p50": pct([t["pickup"] for t in ts], 50),
        "pickup_ms_p99": pct([t["pickup"] for t in ts], 99),
        "turn_ms_p50": pct([t["turn"] for t in ts], 50),
        "overhead_share_pct": round(100 * sum(t["overhead"] for t in ts) / max(1, sum(t["turn"] for t in ts)), 1),
    }


def peak_running_turns(ts: list[dict]) -> int:
    """Most turns in progress at the same moment."""
    edges = sorted([(t["start"], 1) for t in ts] + [(t["start"] + t["turn"], -1) for t in ts])
    running = peak = 0
    for _, step in edges:
        running += step
        peak = max(peak, running)
    return peak


def top_statements(pg, turns_done: int, limit: int = 12) -> list[dict]:
    """The statements costing the most per turn, by calls, by time and by rows
    returned (history reads show up there), since the load began."""
    n = max(1, turns_done)
    out = []
    for order in ("calls", "total_exec_time", "rows"):
        rows = pg.psql("stress", (
            "SELECT calls, total_exec_time, rows, left(regexp_replace(query, '\\s+', ' ', 'g'), 160) "
            f"FROM pg_stat_statements ORDER BY {order} DESC LIMIT {limit}"))
        for calls, ms, returned, query in rows:
            out.append({"by": order, "calls_per_turn": round(float(calls) / n, 1),
                        "ms_per_turn": round(float(ms) / n, 2),
                        "rows_per_turn": round(float(returned) / n, 1), "query": query})
    return out


def background(idle_before: dict, idle_after: dict, seconds: float) -> dict:
    """Counter rates with no load: lease renewals, sweeps and pollers of every process."""
    return {k: (idle_after[k] - idle_before[k]) / max(1e-6, seconds) for k in idle_before}


def per_turn(before: dict, after: dict, turns_done: int, rate: dict | None = None, seconds: float = 0.0) -> dict:
    """Counter deltas per turn; with an idle rate, minus what the run would have cost idle."""
    n = max(1, turns_done)
    d = {k: after[k] - before[k] - (rate or {}).get(k, 0.0) * seconds for k in before}
    out = {
        # Every transaction, read-only and autocommit ones included; WAL syncs
        # are what the disk pays for.
        "commits_per_turn": round(d["commits"] / n, 1),
        "wal_kb_per_turn": round(d["wal_bytes"] / 1024 / n, 1),
        "wal_syncs_per_turn": round(d["wal_syncs"] / n, 1),
        "wal_sync_ms_per_turn": round(d["wal_sync_ms"] / n, 2),
        "statements_per_turn": round(d["statements"] / n, 1),
        "statement_ms_per_turn": round(d["statement_ms"] / n, 1),
    }
    for k in d:
        if k.startswith("cpu_"):
            out[f"{k}_ms_per_turn"] = round(1000 * d[k] / n, 1)
    return out


# Log lines that mean the platform lost work or a connection, even when every
# turn the driver watched finished: a turn whose lease is lost restarts from its
# log, so the driver only sees it as slower. Each counts as an incident and
# fails the run. Plain substrings, matched after ANSI colors are stripped.
INCIDENTS = {
    "lease_lost": "Actor lease lost",
    "lease_renew_failed": "Actor lease renew failed",
    "turn_sealed": "Sealing a session turn",
    "grpc_goaway": "GoAway(",
    "grpc_transport_error": "transport error",
    "db_deadlock": "deadlock detected",
}
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def incidents(log_paths: list[str]) -> dict[str, int]:
    counts = {name: 0 for name in INCIDENTS}
    for path in log_paths:
        try:
            f = open(path, errors="replace")
        except OSError:
            continue
        with f:
            for line in f:
                line = ANSI.sub("", line)
                for name, needle in INCIDENTS.items():
                    if needle in line:
                        counts[name] += 1
    return counts


# Per-turn budget for 8k parallel agents on today's production (2026-11-07):
# capacity, run at 70% at most, divided by the turn rate. The load-test mix
# runs a turn about every 36 s per agent (31 s turn + 5 s think), so 8k agents
# are 222 turns/s with about 6,900 in progress. A run that keeps every cost
# under its budget fits 8k without bigger machines. Local CPU stands in for
# RDS/EKS vCPUs (db.m7g.large vCPUs are physical Graviton cores), so the CPU
# rows are indicative; statements and WAL counts carry over exactly.
TURNS_PER_S_8K = 8000 / 36
RUNNING_TURNS_8K = 8000 * 31 / 36
UTILIZATION = 0.7
PROD = {
    "db_vcpu": 2,            # RDS db.m7g.large
    "db_iops": 3000,         # gp3 baseline, 40 GB
    "db_wal_mib_s": 50,      # of gp3's 125 MiB/s, leaving the rest for data pages
    "app_vcpu": 28,          # Karpenter pool limit 32, less ~4 for Caddy/NATS/UI/Valkey
    "worker_mib": 3 * 6 * 1024 * 0.85,  # 3 workers x 6 GiB, admission brakes at 85% RSS
    "nats_replay_mib": 600,  # of NATS's 1 GiB pod, for a 2-minute replay window
}
BUDGET_8K = {
    # name: (result key, budget per turn, unit)
    "db_cpu": ("cpu_postgres_ms_per_turn", 1000 * PROD["db_vcpu"] * UTILIZATION / TURNS_PER_S_8K, "ms"),
    # ~0.2 ms of database CPU per statement today, so the CPU budget in statements.
    "db_statements": ("statements_per_turn", 30, ""),
    "db_wal_syncs": ("wal_syncs_per_turn", PROD["db_iops"] * UTILIZATION / TURNS_PER_S_8K, ""),
    "db_wal": ("wal_kb_per_turn", 1024 * PROD["db_wal_mib_s"] / TURNS_PER_S_8K, "KB"),
    "app_cpu": ("app_cpu_ms_per_turn", 1000 * PROD["app_vcpu"] * UTILIZATION / TURNS_PER_S_8K, "ms"),
    "worker_memory": ("worker_mib_per_running_turn", PROD["worker_mib"] / RUNNING_TURNS_8K, "MiB"),
    "nats_replay": ("nats_kb_per_turn", 1024 * PROD["nats_replay_mib"] / (120 * TURNS_PER_S_8K), "KB"),
}


def budget(result: dict) -> dict:
    """Each per-turn cost against its 8k budget; `fits` is false when any measured cost is over."""
    rows = {}
    for name, (key, limit, unit) in BUDGET_8K.items():
        value = result.get(key)
        rows[name] = {"value": value, "budget": round(limit, 1), "unit": unit,
                      "x_over": None if value is None else round(value / limit, 1)}
    measured_rows = [r for r in rows.values() if r["value"] is not None]
    return {"rows": rows, "fits": bool(measured_rows) and all(r["value"] <= r["budget"] for r in measured_rows)}
