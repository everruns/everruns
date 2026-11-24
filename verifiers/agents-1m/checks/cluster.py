"""L1 cluster simulation: the whole 1M fleet on machines, over the goal's run.

A small time-stepped model of the physical layout (layout.toml): shard node
pods and executor pods with their memory limits and CPU, spread over zones,
packed onto machines of one type. The load follows the goal: the ramp, the
steady pace, the 3x peak, coding sessions going idle and unloading, then a
zone lost for its minutes. Each pod's memory and CPU per session, per running
turn and per turn come from measurements (results.jsonl), not guesses.

It reports the peak utilization of every pod kind before and after the zone
loss, which pod would be killed, the machines and money the fleet needs, and
the first limit on the way (the "next limit"). D8 = the worst of
peak/70% and after-zone-loss/85%.
"""

import math

from . import Check, Output
from . import model as M

STEP_S = 30


def timeline(cfg):
    """(t seconds, phase) for ramp, steady, peak and zone loss."""
    p, f = cfg.goal["pace"], cfg.goal["failures"]
    ramp, steady, peak, zone = (p["ramp_minutes"] * 60, p["steady_minutes"] * 60,
                                p["peak_minutes"] * 60, f["zone_loss_minutes"] * 60)
    total = ramp + steady + peak + zone
    for t in range(0, total + 1, STEP_S):
        if t < ramp:
            yield t, "ramp", t / ramp
        elif t < ramp + steady:
            yield t, "steady", 1.0
        elif t < ramp + steady + peak:
            yield t, "peak", 1.0
        else:
            yield t, "zone_loss", 1.0


def simulate(cfg, inp, profile, agents=None):
    lay, lim = cfg.layout, cfg.goal["limits"]
    steady = M.load(cfg, inp, profile, agents)
    peak = M.load(cfg, inp, profile, agents, peak=True)
    shards = steady["shards"]
    zones = lay["zones"]
    nodes = shards * lay["nodes_per_shard"]
    turn_s = inp["turn_seconds_in_flight"]
    # Executors are stateless and sized like a planner would: enough for the
    # peak's running turns and to keep CPU under the peak line, and under the
    # zone-loss line with a zone's executors gone.
    peak_running = peak["work_per_sec"] * turn_s
    peak_cores = peak["work_per_sec"] * inp["executor_core_ms_per_turn"] / 1000
    need = max(peak_running / lay["executor_turns_max"],
               peak_cores / (lay["executor_cpu_cores"] * lim["fleet_peak_util"]),
               steady["work_per_sec"] * inp["executor_core_ms_per_turn"] / 1000
               / (lay["executor_cpu_cores"] * lim["fleet_zone_loss_util"] * (1 - 1 / zones)))
    executors = max(zones, math.ceil(need / zones) * zones)
    rows = []
    for t, phase, ramp in timeline(cfg):
        ld = peak if phase == "peak" else steady
        lost = 1 / zones if phase == "zone_loss" else 0.0
        work = ld["work_per_sec"] * ramp
        running = work * turn_s
        resident = steady["resident_session_equivalents"] * ramp / shards
        node_mb = inp.get("node_base_mb", 120) + resident / 1000 * inp["node_mb_per_1k_sessions"]
        # A lost zone takes one replica of each shard and a third of the
        # executors; the survivors carry the shard's CPU and the turns.
        node_cpu = work / shards * inp["shard_core_ms_per_turn"] / 1000 / (lay["nodes_per_shard"] * (1 - lost))
        live_exec = executors * (1 - lost)
        exec_mb = running / live_exec * inp["executor_mb_per_running_turn"]
        exec_cpu = work * inp["executor_core_ms_per_turn"] / 1000 / live_exec
        rows.append({
            "t": t, "phase": phase,
            "node_mem": node_mb / lay["node_memory_limit_mb"],
            "node_cpu": node_cpu / lay["node_cpu_cores"],
            "exec_mem": exec_mb / lay["executor_memory_limit_mb"],
            "exec_cpu": exec_cpu / lay["executor_cpu_cores"],
        })
    # Machines: pods' requests packed per zone (memory limit, CPU request).
    m = lay["machines"][lay["machine"]]
    cpu_req = nodes * lay["node_cpu_cores"] + executors * lay["executor_cpu_cores"]
    mem_req = nodes * lay["node_memory_limit_mb"] + executors * lay["executor_memory_limit_mb"]
    per_zone = max(math.ceil(cpu_req / zones / m["vcpus"]), math.ceil(mem_req / zones / m["memory_mb"]))
    machines = per_zone * zones
    cost = M.fleet_cost(cfg, inp, steady)
    worst = {}
    for kind in ("node_mem", "node_cpu", "exec_mem", "exec_cpu"):
        worst[kind] = {
            "peak": max(r[kind] for r in rows if r["phase"] != "zone_loss"),
            "zone_loss": max(r[kind] for r in rows if r["phase"] == "zone_loss"),
        }
    d8 = max(max(w["peak"] for w in worst.values()) / lim["fleet_peak_util"],
             max(w["zone_loss"] for w in worst.values()) / lim["fleet_zone_loss_util"])
    killed = [k for k, w in worst.items() if k.endswith("mem") and max(w.values()) > 1.0]
    return {
        "agents": steady["agents"], "shards": shards, "node_pods": nodes, "executor_pods": executors,
        "machines": {lay["machine"]: machines}, "vcpus": machines * m["vcpus"],
        "memory_gb": round(machines * m["memory_mb"] / 1024),
        "machines_usd_per_hour": round(machines * m["usd_per_hour"], 2),
        "store_usd_per_hour": round(cost["store_usd_per_hour"], 2),
        "worst": {k: {p: round(v, 3) for p, v in w.items()} for k, w in worst.items()},
        "d8": round(d8, 3), "killed": killed,
        "timeline": [{k: (round(v, 3) if isinstance(v, float) else v) for k, v in r.items()}
                     for r in rows[:: max(1, len(rows) // 40)]],
    }


LABELS = {"node_mem": "shard node memory", "node_cpu": "shard node CPU",
          "exec_mem": "executor memory", "exec_cpu": "executor CPU"}


def next_limit(cfg, sim):
    """The first line crossed on the way to the goal, in words."""
    lim = cfg.goal["limits"]
    over = []
    for kind, w in sim["worst"].items():
        for phase, line in (("peak", lim["fleet_peak_util"]), ("zone_loss", lim["fleet_zone_loss_util"])):
            if w[phase] > line:
                over.append((w[phase] / line, kind, phase, w[phase], line))
    if not over:
        return None
    # A pod over its memory limit is killed, which outranks CPU running hot.
    kills = [o for o in over if o[1].endswith("mem") and o[3] > 1.0]
    ratio, kind, phase, value, line = max(kills or over)
    when = "at the 3x peak" if phase == "peak" else "after losing a zone"
    return (f"{LABELS[kind]} reaches {value:.0%} of its pod's {'limit' if kind.endswith('mem') else 'CPU'} "
            f"{when} (line {line:.0%}) at any fleet size: per-shard density is fixed, so adding "
            f"shards does not help; lower the cost per session or turn, or change layout.toml (D8, simulated)")


def run(ctx):
    cfg = ctx.cfg
    out = Output()
    inp = {k: v["value"] for k, v in M.measured(cfg, ctx.runs).items()}
    need = ["node_mb_per_1k_sessions", "executor_mb_per_running_turn", "turn_seconds_in_flight",
            "shard_core_ms_per_turn", "executor_core_ms_per_turn", "store_puts_per_turn"]
    missing = [k for k in need if k not in inp]
    if missing:
        out.notes.append("cluster simulation skipped, inputs not measured: " + ", ".join(missing))
        return out
    sim = simulate(cfg, inp, ctx.profile)
    out.add("D8", "L1", sim["d8"], note=(
        f"{sim['machines']} for {sim['node_pods']} shard node pods + {sim['executor_pods']} executors"
        + (f"; would be killed: {', '.join(sim['killed'])}" if sim["killed"] else "")))
    out.metrics["resources"] = {k: v for k, v in sim.items() if k not in ("timeline",)}
    out.metrics["next_limit"] = next_limit(cfg, sim)
    out.metrics["cluster_timeline"] = sim["timeline"]
    for n in cfg.goal["milestones"]["agents"]:
        s = simulate(cfg, inp, ctx.profile, agents=n)
        out.metrics.setdefault("resources_by_milestone", {})[str(n)] = {
            "machines": s["machines"], "vcpus": s["vcpus"], "memory_gb": s["memory_gb"],
            "usd_per_hour": round(s["machines_usd_per_hour"] + s["store_usd_per_hour"], 2)}
    return out


CHECKS = [Check("cluster", "L1", ["D8"], minutes=0.1,
                command="verify (runs with every default run)", run=run)]
