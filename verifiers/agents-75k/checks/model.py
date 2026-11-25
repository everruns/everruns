"""L0 model: arithmetic from measured per-agent costs.

Inputs are the newest measured per-agent costs in results.jsonl (highest
level first): memory per 1,000 sessions, executor memory per running turn,
turn time in flight, CPU per turn and object store writes per turn. Shard
capacity adds up linearly over the goal's few shards. Missing inputs are reported, never guessed;
the two assumptions below are named in every report until measured.

Coding agents (25% of the goal) take turns at the pace in the coding
profile; a coding turn counts as `calls / ASSUMED mix calls` mix turns of
CPU, and a resident coding session as `coding_memory_factor` mix sessions.
"""

import json
import os

from . import Check, Output

PROFILE = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                       "workload", "coding-profile.json")

# Until G1 and G2 measure them on agentd.
ASSUMPTIONS = {
    "mix_model_calls_per_turn": 3.0,   # the 75k feature mix's llmsim scenarios average about 3 model calls
    "coding_memory_factor": 4.0,       # a resident coding session holds ~4x a mix session's history
}

INPUT_KEYS = [
    "node_mb_per_1k_sessions", "node_base_mb", "executor_mb_per_running_turn",
    "turn_seconds_in_flight", "shard_core_ms_per_turn", "executor_core_ms_per_turn",
    "store_puts_per_turn",
]
LEVEL_RANK = {"L4": 4, "L3": 3, "L2": 2, "L1": 1, "L0": 0}


def measured(cfg, runs):
    """key -> {value, run, level}: the best-level, newest measurement of each input."""
    out = {}
    for run in sorted(runs, key=lambda r: r["at"]):
        if not cfg.accepts(run.get("fingerprint")) or run.get("invalid"):
            continue
        level = max((LEVEL_RANK[l] for l in run.get("levels", [])), default=0)
        if level == 0:
            continue  # the model does not feed itself
        for k, v in (run.get("metrics") or {}).items():
            if k not in INPUT_KEYS or v is None:
                continue
            if k not in out or level >= out[k]["rank"]:
                out[k] = {"value": v, "run": run["run"], "level": f"L{level}", "rank": level}
    return out


def load_profile(path=PROFILE):
    with open(path) as f:
        return json.load(f)


def coding_pace(cfg, profile):
    """Turn interval, calls per turn and the resident share of a coding session."""
    sh = profile["shapes"]
    turn_s = sh["turn_seconds"]["mean"]
    gap_s = sh["gap_minutes"]["mean"] * 60
    idle = cfg.layout["idle_unload_s"]
    every = turn_s + gap_s
    # Resident while running and for idle_unload_s after (at most the gap).
    resident = min(1.0, (turn_s + min(gap_s, idle)) / every) if every else 1.0
    return {"turn_every_s": every, "calls_per_turn": sh["model_calls_per_turn"]["mean"],
            "resident_share": resident}


def load(cfg, inp, profile, agents=None, peak=False):
    """Rates for the goal fleet in mix-turn equivalents."""
    d = cfg.derived(agents)
    g = cfg.goal
    share = g["agents"]["coding_share"]
    cp = coding_pace(cfg, profile)
    mult = g["pace"]["peak_multiplier"] if peak else 1
    mix_rate = d["agents"] * (1 - share) / g["pace"]["turn_every_s"] * mult
    coding_rate = d["agents"] * share / cp["turn_every_s"] * mult
    factor = cp["calls_per_turn"] / ASSUMPTIONS["mix_model_calls_per_turn"]
    work = mix_rate + coding_rate * factor
    resident = d["agents"] * (1 - share) + d["agents"] * share * cp["resident_share"] * ASSUMPTIONS["coding_memory_factor"]
    return {"turns_per_sec": mix_rate + coding_rate, "work_per_sec": work,
            "resident_session_equivalents": resident, "shards": d["shards"], "agents": d["agents"],
            "coding": cp, "coding_cpu_factor": factor}


# Universal Scalability Law: X(N) = l N / (1 + s (N - 1) + k N (N - 1)).
def fleet_cost(cfg, inp, ld):
    """USD per hour for the goal fleet at the steady pace: pods' CPU at the
    machine's price per vCPU, plus object store writes."""
    lay = cfg.layout
    m = lay["machines"][lay["machine"]]
    per_vcpu = m["usd_per_hour"] / m["vcpus"]
    shard_cores = ld["shards"] * lay["nodes_per_shard"] * lay["node_cpu_cores"]
    exec_cores = ld["work_per_sec"] * inp["executor_core_ms_per_turn"] / 1000 / (1 - cfg.goal["limits"]["headroom"])
    compute = (shard_cores + exec_cores) * per_vcpu
    # Store writes follow turns, not model calls (history segments, log).
    store = ld["turns_per_sec"] * inp["store_puts_per_turn"] * 3600 * lay["store"]["usd_per_1k_puts"] / 1000
    return {"compute_usd_per_hour": compute, "store_usd_per_hour": store,
            "usd_per_hour": compute + store, "usd_per_agent_hour": (compute + store) / ld["agents"]}


def run(ctx):
    cfg = ctx.cfg
    out = Output()
    got = measured(cfg, ctx.runs)
    inp = {k: v["value"] for k, v in got.items()}
    missing = [k for k in INPUT_KEYS if k not in inp]
    out.metrics["model_inputs"] = {k: {"value": v["value"], "from": v["run"], "level": v["level"]}
                                   for k, v in got.items()}
    out.metrics["model_assumptions"] = dict(ASSUMPTIONS)
    if missing:
        out.notes.append("model inputs not measured yet: " + ", ".join(missing))
    profile = ctx.profile
    ld = load(cfg, inp, profile)
    pk = load(cfg, inp, profile, peak=True)
    out.metrics["load"] = {"turns_per_sec": round(ld["turns_per_sec"], 1),
                           "work_per_sec": round(ld["work_per_sec"], 1),
                           "peak_work_per_sec": round(pk["work_per_sec"], 1),
                           "coding_cpu_factor": round(ld["coding_cpu_factor"], 2)}
    goal_turns = cfg.derived()["turns_per_sec"]
    if "shard_core_ms_per_turn" in inp:
        lay = cfg.layout
        per_shard = lay["nodes_per_shard"] * lay["node_cpu_cores"] * 1000 / inp["shard_core_ms_per_turn"]
        n = ld["shards"]
        capacity_work = per_shard * n  # linear: growing past 75k is the 1M verifier's question
        capacity = goal_turns * capacity_work / ld["work_per_sec"]
        out.add("B1", "L0", round(capacity, 1),
                note=f"{per_shard:,.0f} work turns/s per shard x {n} shards")
        out.metrics["capacity_turns_per_sec"] = round(capacity, 1)
    if all(k in inp for k in ("executor_core_ms_per_turn", "store_puts_per_turn")):
        cost = fleet_cost(cfg, inp, ld)
        out.add("C6", "L0", cost["usd_per_agent_hour"],
                note=f"${cost['compute_usd_per_hour']:,.0f}/h compute + ${cost['store_usd_per_hour']:,.0f}/h store writes")
        out.metrics["cost"] = {k: round(v, 7 if "agent" in k else 2) for k, v in cost.items()}
    return out


CHECKS = [Check("model", "L0", ["B1", "C6"], minutes=0.1,
                command="verify (runs with every default run)", run=run)]
