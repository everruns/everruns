"""L2 local adapters: the agentd harnesses in scripts/agentd/, run with the
goal's numbers on this machine, and their output read into evidence.

Per-shard points count only at the goal's density (25,000 sessions on one
shard), which needs about 30 GB: the Mac mini, not a cloud session. Smaller
runs (`--size 0.1`) still feed the model's inputs and test the plumbing.
Build first: cargo build --release -p everruns-agentd-testkit --bins
"""

import json
import os
import sys

from engine import runners as X
from engine.config import REPO
from . import Check, Output
from . import model as M

SCRIPTS = os.path.join(REPO, "scripts", "agentd")


last_json = X.last_json


# Adapters: pure functions from a harness's output to evidence.

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
    if summary.get("chaos"):
        out.add("E1", "L2", (summary.get("errors") or 0) == 0, note=f"killed {summary['chaos'][0]['killed']}, {summary.get('errors')} errors")
    return out


def scale(results, goal_shards):
    curve = [[r["shards"], r["turns_per_sec"]] for r in results]
    out = Output()
    out.metrics["scale_curve"] = curve
    fit = M.usl_fit(curve)
    if fit:
        peak = M.usl_peak(fit[1], fit[2])
        out.add("D1", "L2", round(min(peak / goal_shards, 99.0), 3),
                note=f"USL over {len(curve)} points: sigma {fit[1]:.4f} kappa {fit[2]:.6f}, peak {peak:,.0f} shards")
    return out


def rejoin(result):
    out = Output()
    note = f"{result.get('errors')} errors, caught up in {result.get('caught_up_secs')} s"
    out.add("E1", "L2", bool(result.get("ok")), note=note)
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


def run_scale(ctx):
    text, cmd = harness(ctx, "scale.py", ["--shards", "1,2,4,8", "--sessions", max(2000, int(20000 * ctx.size)), "--turns", 2], "scale")
    return scale(last_json(text), ctx.cfg.derived()["shards"])


def run_rejoin(ctx):
    text, cmd = harness(ctx, "prove_rejoin_under_load.py", [], "rejoin")
    return rejoin(last_json(text))


def run_many_sessions(ctx):
    n = 2 * density_sessions(ctx)
    text, cmd = harness(ctx, "many_sessions.py", ["--sessions", n, "--shards", 2, "--turns", 2], "many_sessions")
    return many_sessions(last_json(text), 2)


CARGO = ("cargo",)
CHECKS = [
    Check("shard-soak", "L2", ["A4", "E1"], minutes=75, needs=CARGO, run=run_shard_soak,
          command="scripts/agentd/shard_soak.py --scale 1.0"),
    Check("scale", "L2", ["D1"], minutes=40, needs=CARGO, run=run_scale,
          command="scripts/agentd/scale.py --shards 1,2,4,8 --sessions 20000 --turns 2"),
    Check("rejoin", "L2", ["E1"], minutes=5, needs=CARGO, run=run_rejoin,
          command="scripts/agentd/prove_rejoin_under_load.py"),
    Check("many-sessions", "L2", ["A1"], minutes=40, needs=CARGO, run=run_many_sessions,
          command="scripts/agentd/many_sessions.py --sessions 50000 --shards 2 --turns 2"),
]
