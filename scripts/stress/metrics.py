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
import threading
import time

CLK_TCK = os.sysconf("SC_CLK_TCK")


def proc_cpu_seconds(pid: int, children: bool = False) -> float:
    """utime + stime of a process; with children, also of its reaped children."""
    try:
        with open(f"/proc/{pid}/stat") as f:
            fields = f.read().rsplit(")", 1)[1].split()
    except (FileNotFoundError, ProcessLookupError):
        return 0.0
    # Fields after the comm: state is [0], utime [11], stime [12], cutime [13], cstime [14].
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
                if int(f.read().rsplit(")", 1)[1].split()[2]) == parent:
                    out.append(int(entry))
        except (FileNotFoundError, ProcessLookupError, IndexError, ValueError):
            continue
    return out


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
        }


def moving(values: list[float], width: int) -> list[float]:
    return [sum(values[i:i + width]) / width for i in range(max(0, len(values) - width + 1))]


TURN_EVENTS = ("input.message", "turn.started", "llm.generation", "act.completed",
               "turn.completed", "turn.failed", "turn.cancelled")


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
        elif cur and kind in ("turn.completed", "turn.failed", "turn.cancelled"):
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


def top_statements(pg, turns_done: int, limit: int = 12) -> list[dict]:
    """The statements costing the most per turn, by calls and by time, since the load began."""
    n = max(1, turns_done)
    out = []
    for order in ("calls", "total_exec_time"):
        rows = pg.psql("stress", (
            "SELECT calls, total_exec_time, left(regexp_replace(query, '\\s+', ' ', 'g'), 160) "
            f"FROM pg_stat_statements ORDER BY {order} DESC LIMIT {limit}"))
        for calls, ms, query in rows:
            out.append({"by": order, "calls_per_turn": round(float(calls) / n, 1),
                        "ms_per_turn": round(float(ms) / n, 2), "query": query})
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
