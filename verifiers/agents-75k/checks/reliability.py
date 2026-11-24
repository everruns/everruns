"""L2 reliability: the agentd proofs that break a running local fleet on
purpose (scripts/agentd/prove_*.py, chaos.py, soak.py) and check that every
session still finished exactly as it should.

Each harness prints one JSON result last and exits non-zero on a failed
check, so a point passes when the result's verdict and the load's errors
agree. A harness that crashed before printing fails its point with a note,
never silently.

Build first: cargo build --release -p everruns-agentd-testkit --bins
"""

from . import Check, Output
from .local import density_sessions, harness, last_json


def verdict(res):
    """ok from a proof's result: its own ok, a nested check's ok, no errors."""
    ok = res.get("ok")
    if ok is None and "check" not in res:
        # Proofs that check in phases report each (prove_topology.py: check_a, check_b).
        phases = [v for k, v in res.items() if k.startswith("check_") and isinstance(v, dict)]
        if phases:
            ok = all(v.get("ok") for v in phases)
    if ok is None:
        check = res.get("check")
        if isinstance(check, dict) and "ok" in check:
            ok = check["ok"]
        elif isinstance(check, dict):
            ok = all((c or {}).get("ok") for c in check.values()) if check else False
    errors = res.get("errors", res.get("load_errors"))
    if errors is None:
        errors = sum((v.get("errors") or 0) for k, v in res.items() if k.startswith("load_") and isinstance(v, dict))
    errors = errors or 0
    return bool(ok) and errors == 0, errors


def failed_sessions(res):
    check = res.get("check")
    if isinstance(check, dict):
        if "failed_sessions" in check:
            return check.get("failed_sessions") or 0
        return sum((c or {}).get("failed_sessions") or 0 for c in check.values() if isinstance(c, dict))
    return 0


def adapt(point, res, what):
    out = Output()
    ok, errors = verdict(res)
    out.add(point, "L2", ok, note=f"{what}: {errors} errors, {failed_sessions(res)} failed sessions")
    return out


def leader_failover(res):
    out = Output()
    out.add("R1", "L2", bool(res.get("ok")),
            note=f"new leader in {res.get('election_secs')} s, {res.get('model_calls_started')} of "
                 f"{res.get('expected_model_calls')} model calls (none repeated), old leader caught up: {res.get('restarted_leader_caught_up')}")
    return out


def density(res):
    """Sessions per shard the proof ran at: its fullest shard's seeding
    (`fleet.seed`), so a proof counts for per-shard points only at goal
    density; none reported reads as 0."""
    held = res.get("sessions_per_shard") or {}
    return min(held.values()) if held else 0


def proof(point, script, args, what, minutes, seed=True):
    def run(ctx):
        full = [*args, "--seed-per-shard", density_sessions(ctx)] if seed else list(args)
        text, cmd = harness(ctx, script, full, point.lower() + "-" + script.removesuffix(".py"))
        try:
            res = last_json(text)
        except ValueError:
            out = Output()
            out.add(point, "L2", False, note=f"{script} printed no result (crashed? see its log)", evidence=cmd)
            return out
        out = leader_failover(res) if point == "R1" else adapt(point, res, what)
        out.density = density(res)
        for e in out.entries:
            e["evidence"] = cmd
            e["density"] = out.density
        return out
    command = "scripts/agentd/" + " ".join([script, *map(str, args)])
    return Check(script.removesuffix(".py").replace("_", "-") + ("-executors" if "--executor-kills" in args else ""),
                 "L2", [point], minutes=minutes, needs=("cargo",), run=run, command=command)


CHECKS = [
    proof("R1", "prove_leader_failover.py", [], "leader killed mid model call", 5),
    proof("R2", "prove_follower_catchup.py", [], "lagging follower healed from a snapshot", 10, seed=False),
    proof("R4", "chaos.py", ["--shards", 2, "--sessions", 300, "--turns", 3, "--kills", 15, "--moves", 100],
          "15 random node kills with 100 live moves", 15),
    proof("R5", "chaos.py", ["--shards", 2, "--executors", 3, "--executor-kills", 10],
          "10 executor kills mid-turn", 15),
    proof("R6", "prove_rebalance.py", [], "sessions moved mid-turn by the rebalancer", 10, seed=False),
    proof("R7", "prove_scale_in.py", [], "a shard drained and retired under long turns", 10, seed=False),
    proof("R8", "prove_topology.py", [], "nodes and shards added and removed under load", 10, seed=False),
    proof("R9", "soak.py", ["--minutes", 60, "--rate", 20, "--think", 20, "--turns", 5, "--kill-every", 300],
          "an hour of production-shaped load with a node killed every 5 minutes", 70),
    proof("U11", "prove_store_outage.py", ["--sessions", 1000, "--outage", 600],
          "object store down for 10 minutes, then the shard lost and rebuilt from the store", 20),
]
