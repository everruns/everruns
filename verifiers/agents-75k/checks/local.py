"""L2 local adapters: the agentd harnesses in scripts/agentd/, run with the
goal's numbers on this machine, and their output read into evidence.

Per-shard points count only at the goal's density (25,000 sessions on one
shard), which needs about 30 GB: the Mac mini, not a cloud session. Smaller
runs (`--size 0.1`) still feed the model's inputs and test the plumbing.
Build first: cargo build --release -p everruns-agentd-testkit --bins
"""

import json
import os
import statistics
import sys

from engine import runners as X
from engine.config import REPO
from . import Check, Output

SCRIPTS = os.path.join(REPO, "scripts", "agentd")


last_json = X.last_json


def slope_per_hour(points):
    """Least-squares slope of (t seconds, value) as a share of the mean per hour."""
    if len(points) < 4:
        return None
    ts = [t for t, _ in points]
    vs = [v for _, v in points]
    mt, mv = statistics.fmean(ts), statistics.fmean(vs)
    den = sum((t - mt) ** 2 for t in ts)
    if den == 0 or mv == 0:
        return None
    slope = sum((t - mt) * (v - mv) for t, v in points) / den
    return slope * 3600 / mv


# Adapters: pure functions from a harness's output to evidence.

def memory_bench(result):
    out = Output(density=result["sessions"])
    g = result["gated"]
    out.add("C1", "L2", g["node_rss_mb_per_1k_sessions"], note=f"{result['sessions']:,} sessions on one shard, {result.get('profile')} build")
    out.add("C5", "L2", g["puts_per_turn"])
    idle = result.get("rss_mb", {}).get("idle", {})
    out.metrics.update({
        "node_mb_per_1k_sessions": g["node_rss_mb_per_1k_sessions"],
        "store_puts_per_turn": g["puts_per_turn"],
    })
    if idle:
        out.metrics["node_base_mb"] = round(statistics.fmean(idle.values()))
    return out


def shard_soak(summary, samples):
    cfg = summary["config"]
    out = Output(density=cfg["sessions"])
    # Decision: a crash fails the gate like an out-of-memory kill; a controlled
    # restart (exit 75, before the limit) is counted apart, as the soak does.
    ooms, crashes = len(summary["oom_kills"]), len(summary.get("crashes") or [])
    restarts = summary.get("restarts") or []
    out.add("A4", "L2", ooms + crashes,
            note=f"limit {cfg['limit_mb']} MB, peak share {summary['peak_rss_share_of_limit']}, "
                 f"{ooms} out-of-memory kills, {crashes} crashes, {len(restarts)} controlled restarts, "
                 f"snapshot every {cfg.get('snapshot_every', '?')} entries")
    peak = summary["peak_rss_mb"]
    # The memory ledger's remainder (what no source accounts for) when the
    # harness reports it; before the ledger, the resident set less events and log.
    held = summary.get("peak_remainder_mb") or summary["peak_unaccounted_mb"]
    shares = [held[n] / peak[n] for n in peak if peak[n] and n in held]
    if shares:
        out.add("C3", "L2", round(max(shares), 3),
                note="memory ledger remainder" if summary.get("peak_remainder_mb") else "resident set less events and Raft log")
    killed = {k.split()[-1] for c in summary.get("chaos", []) for k in c["killed"] if k.startswith("node")}
    if summary.get("chaos"):
        out.add("E1", "L2", (summary.get("errors") or 0) == 0, note=f"killed {summary['chaos'][0]['killed']}, {summary.get('errors')} errors")
    creep = []
    if samples:
        start = cfg.get("ramp", 600) + 300
        end = 0.8 * samples[-1]["t"]
        for n in samples[0]["nodes"]:
            if n in killed:
                continue
            pts = [(r["t"], r["nodes"][n]["rss_mb"]) for r in samples if start <= r["t"] <= end and r["nodes"][n]["rss_mb"]]
            s = slope_per_hour(pts)
            if s is not None:
                creep.append(s)
    if creep:
        out.add("F1", "L2", round(max(0.0, max(creep)), 4), note="steady window, nodes not killed")
    # The Raft log's peak share of the soak's memory limit: below goal density
    # the verifier counts a soak only when this is near the gate's
    # (levels.toml [L2.scaled]).
    logs = summary.get("peak_log_mb") or {}
    if logs and cfg.get("limit_mb"):
        share = round(max(logs.values()) / cfg["limit_mb"], 4)
        for e in out.entries:
            e["log_share"] = share
    return out


def bench(result):
    out = Output(density=result["sessions"])
    shard = result["leader_cpu_ms_per_turn"] + 2 * result["follower_cpu_ms_per_turn"]
    out.add("C4", "L2", round(shard, 2), note=f"{result['agent']} agent, {result['sessions']} sessions")
    out.metrics.update({"shard_core_ms_per_turn": round(shard, 2),
                        "executor_core_ms_per_turn": result["executor_cpu_ms_per_turn"]})
    return out


def drill(res):
    out = Output(density=res["sessions"] / res["shards"])
    ok = all((c or {}).get("ok") for c in res["check"].values())
    # E2 passes when the shard is rebuilt and every session finishes, resends
    # included; messages lost in the shipping window are judged only by U10
    # (owner, 2026-11-25), so one loss is not counted twice.
    out.add("E2", "L2", bool(ok) and bool(res.get("rebuilt")),
            note=f"rebuild {res['rebuild_s']} s, leader back {res['kill_to_leader_s']} s, {res['second_messages_lost']} second messages resent")
    if res.get("ship_lag_s", {}).get("p99") is not None:
        out.add("F3", "L2", res["ship_lag_s"]["p99"])
    # The 75k verifier's durability points read the same drill in detail.
    if res.get("rebuilt"):
        out.add("U7", "L2", res["rebuild_s"], note=f"shard of about {res['sessions'] // res['shards']:,} sessions rebuilt from the store")
    out.add("U8", "L2", res["kill_to_leader_s"], note="whole shard killed to a leader again, rebuild included")
    if (res.get("ship_first_s") or {}).get("p99") is not None:
        out.add("U9", "L2", res["ship_first_s"]["p99"])
    if res.get("inflight"):
        out.add("U10", "L2", round(res["second_messages_lost"] / res["inflight"], 4),
                note=f"{res['second_messages_lost']} of {res['inflight']} in-flight second messages resent after the rebuild")
    return out


def scale(results):
    """B6: turns/s at the most shards over shards x turns/s at one shard. A
    run with errors scores 0: throughput bought by dropping turns is not
    scaling."""
    out = Output()
    curve = sorted((r["shards"], r["turns_per_sec"]) for r in results)
    out.metrics["scale_curve"] = [list(p) for p in curve]
    one = dict(curve).get(1)
    n, top = curve[-1]
    if not one or n < 2:
        return out
    errors = sum(r.get("errors") or 0 for r in results)
    eff = 0.0 if errors else top / (n * one)
    out.add("B6", "L2", round(eff, 3),
            note=f"{one:,.0f} turns/s on 1 shard, {top:,.0f} on {n}" + (f", {errors} errors" if errors else ""))
    return out


def ramp(res):
    """B3: sessions created a second at the goal's ramp rate. A run with
    errors scores 0: creates that failed are not a ramp."""
    out = Output()
    ok = (res.get("errors") or 0) == 0
    out.add("B3", "L2", round(res.get("created_per_sec") or 0, 2) if ok else 0,
            note=f"{res.get('offered_per_sec')}/s offered, create p99 {res.get('create_p99_ms')} ms, "
                 f"{res.get('shed')} refused and retried, {res.get('errors')} errors")
    return out


def peak(res):
    """B2: turns served a second at three times the pace. It counts only
    when under 0.1% of requests were shed and none failed (the point's
    title): a peak served by refusing work is not served."""
    out = Output()
    ok = (res.get("errors") or 0) == 0 and (res.get("shed_share") or 0) <= 0.001
    out.add("B2", "L2", res.get("turns_per_sec") or 0 if ok else 0,
            note=f"{res.get('offered_turns_per_sec')} turns/s offered, {res.get('turns_per_sec')} served, "
                 f"{res.get('shed_share', 0):.3%} shed, {res.get('errors')} errors")
    return out


def rejoin(result):
    held = result.get("sessions_per_shard") or {}
    out = Output(density=min(held.values()) if held else 0)
    note = f"{result.get('errors')} errors, caught up in {result.get('caught_up_secs')} s"
    out.add("E1", "L2", bool(result.get("ok")), note=note)
    out.add("R3", "L2", bool(result.get("ok")), note=note)
    return out


def many_sessions(res, shards):
    out = Output(density=res["sessions"] / shards)
    ok = bool(res["check"].get("ok")) and res["errors"] == 0
    out.add("A1", "L2", ok, note=f"{res['sessions']:,} sessions x turns checked: {res['check'].get('failed_sessions') or 0} failed")
    return out


# Runners.

def harness(ctx, script, args, name):
    cmd = [sys.executable, os.path.join(SCRIPTS, script), *map(str, args)]
    # The release build this module's notes ask for: scripts that would
    # otherwise default to target/debug (memory_bench.py) use it too.
    env = dict(os.environ)
    env.setdefault("AGENTD_BIN_DIR", os.path.join("target", "release"))
    text, _ = X.run_logged(cmd, os.path.join(ctx.evidence, f"{name}.log"), env=env)
    return text, " ".join(cmd[1:])


def density_sessions(ctx):
    return max(200, int(ctx.cfg.goal["density"]["sessions_per_shard"] * ctx.size))


def coding_memory(result):
    """G1: node memory per thousand coding sessions, the worst node."""
    out = Output(density=result["sessions"])
    # Decision (coordinator, 2026-11-25): a pass needs no errors and every
    # turn done; the note records the per-node limit and how many sessions
    # were unloaded, so a run that only fit by unloading everything shows.
    expected = result["sessions"] * (result.get("turns_per_session") or 0)
    ok = (result.get("errors") or 0) == 0 and result.get("turns") == expected
    out.add("G1", "L2", result["gated"]["node_rss_mb_per_1k_sessions"] if ok else 1e9,
            note=f"{result['sessions']:,} coding sessions on one shard, {result.get('turns_per_session')} turns each, "
                 f"{result.get('turns')}/{expected} turns, {result.get('errors')} errors, "
                 f"node limit {result.get('limit_mb') or 'none'} MB, {result.get('unloaded_sessions')} sessions unloaded")
    return out


def run_coding_memory(ctx):
    text, cmd = harness(ctx, "memory_bench.py", ["--sessions", density_sessions(ctx), "--turns", 4, "--coding"], "coding_memory")
    out = coding_memory(last_json(text))
    for e in out.entries:
        e["evidence"] = cmd
    return out


def run_memory_bench(ctx):
    text, cmd = harness(ctx, "memory_bench.py", ["--sessions", density_sessions(ctx), "--turns", 4, "--no-baseline"], "memory_bench")
    out = memory_bench(last_json(text))
    for e in out.entries:
        e["evidence"] = cmd
    return out


def snapshot_every(size):
    """Entries between snapshots for a shard soak at `size` of goal density.

    Decision: at full density the gate's own interval (20,000). Below it the
    Raft log would stay a far smaller share of the memory limit than on the
    gate, and a log-driven out-of-memory kill would not show: at 1/8 density
    the gate's interval kept the log near 3% of the limit and the pre-fix
    build passed, while 60,000 put it at 11-14% (gate: about 9%) and
    reproduced the gate's kills (Mac mini, 2026-11-23).
    """
    return 20_000 if size >= 0.95 else 60_000


def run_shard_soak(ctx):
    d = os.path.join(ctx.evidence, "shard-soak")
    _, cmd = harness(ctx, "shard_soak.py", ["--scale", ctx.size, "--snapshot-every", snapshot_every(ctx.size), "--out", d], "shard_soak")
    with open(os.path.join(d, "summary.json")) as f:
        summary = json.load(f)
    with open(os.path.join(d, "samples.jsonl")) as f:
        samples = [json.loads(l) for l in f if l.strip()]
    out = shard_soak(summary, samples)
    for e in out.entries:
        e["evidence"] = cmd
    return out


def run_bench(ctx):
    text, cmd = harness(ctx, "bench.py", ["--sessions", 200, "--turns", 10, "--agent", "chat", "--no-baseline"], "bench")
    return bench(last_json(text))


def run_drill(ctx):
    n = 2 * density_sessions(ctx)
    text, cmd = harness(ctx, "drill.py", ["--sessions", n, "--shards", 2, "--inflight", min(3000, n // 10)], "drill")
    return drill(last_json(text))


def run_scale(ctx):
    text, cmd = harness(ctx, "scale.py", ["--shards", "1,2,3", "--sessions", max(2000, int(20000 * ctx.size)), "--turns", 2], "scale")
    out = scale(last_json(text))
    for e in out.entries:
        e["evidence"] = cmd
    return out


def goal_rates(ctx):
    """The goal's full-size ramp (sessions a second) and peak (turns a
    second, minutes): shared points count only from full-size traffic."""
    g = ctx.cfg.goal
    n, pace = g["agents"]["total"], g["pace"]
    return n / (pace["ramp_minutes"] * 60), n / pace["turn_every_s"] * pace["peak_multiplier"], pace["peak_minutes"]


PEAK_THINK_S = 10


def run_ramp(ctx):
    rate, _, _ = goal_rates(ctx)
    text, cmd = harness(ctx, "load_profile.py", ["ramp", "--rate", round(rate, 2), "--duration", 300], "ramp")
    out = ramp(last_json(text))
    for e in out.entries:
        e["evidence"] = cmd
    return out


def run_peak(ctx):
    _, turns, minutes = goal_rates(ctx)
    sessions = int(turns * PEAK_THINK_S)
    text, cmd = harness(ctx, "load_profile.py", ["peak", "--sessions", sessions, "--think", PEAK_THINK_S,
                                                 "--minutes", minutes], "peak")
    out = peak(last_json(text))
    for e in out.entries:
        e["evidence"] = cmd
    return out


def run_rejoin(ctx):
    text, cmd = harness(ctx, "prove_rejoin_under_load.py", ["--seed-per-shard", density_sessions(ctx)], "rejoin")
    return rejoin(last_json(text))


def run_many_sessions(ctx):
    n = 2 * density_sessions(ctx)
    text, cmd = harness(ctx, "many_sessions.py", ["--sessions", n, "--shards", 2, "--turns", 2], "many_sessions")
    return many_sessions(last_json(text), 2)


CARGO = ("cargo",)
CHECKS = [
    Check("memory-bench", "L2", ["C1", "C5"], minutes=60, needs=CARGO, run=run_memory_bench,
          command="scripts/agentd/memory_bench.py --sessions 25000 --turns 4 --no-baseline"),
    Check("coding-memory", "L2", ["G1"], minutes=60, needs=CARGO, run=run_coding_memory,
          command="scripts/agentd/memory_bench.py --sessions 25000 --turns 4 --coding"),
    # U13 comes from its own proof (`prove_memory_restart.py`): asking for
    # it must not start a full-size soak, which no node of a local run holds.
    Check("shard-soak", "L2", ["A4", "C3", "E1", "F1"], minutes=75, needs=CARGO, run=run_shard_soak,
          command="scripts/agentd/shard_soak.py --scale 1.0"),
    Check("bench", "L2", ["C4"], minutes=10, needs=CARGO, run=run_bench,
          command="scripts/agentd/bench.py --sessions 200 --turns 10 --agent chat --no-baseline"),
    Check("drill", "L2", ["E2", "F3", "U7", "U8", "U9", "U10"], minutes=30, needs=CARGO, run=run_drill,
          command="scripts/agentd/drill.py --sessions 50000 --shards 2 --inflight 3000"),
    Check("scale", "L2", ["B6"], minutes=30, needs=CARGO, run=run_scale,
          command="scripts/agentd/scale.py --shards 1,2,3 --sessions 20000 --turns 2"),
    Check("ramp", "L2", ["B3"], minutes=10, needs=CARGO, run=run_ramp,
          command="scripts/agentd/load_profile.py ramp --rate 41.67 --duration 300"),
    Check("peak", "L2", ["B2"], minutes=15, needs=CARGO, run=run_peak,
          command="scripts/agentd/load_profile.py peak --sessions 6250 --think 10 --minutes 10"),
    Check("rejoin", "L2", ["E1", "R3"], minutes=5, needs=CARGO, run=run_rejoin,
          command="scripts/agentd/prove_rejoin_under_load.py"),
    Check("many-sessions", "L2", ["A1"], minutes=40, needs=CARGO, run=run_many_sessions,
          command="scripts/agentd/many_sessions.py --sessions 50000 --shards 2 --turns 2"),
]
