"""L3/L4 cloud: the SaaS data plane load workflow (everruns/saas
.github/workflows/dataplane-load.yml) on EKS.

Two halves:
- import: read a finished run's artifact (summary.json, samples.jsonl,
  usage.jsonl, chaos.log) into evidence. A run at the goal's density per
  shard counts for the per-shard points at cloud certainty; shared points
  count only from a run of the goal's full size (or a milestone's).
- dispatch: start a run with the goal's numbers. Paid, so it refuses
  without --spend and a reason; the full run also needs the owner's
  approval recorded in goal.toml [approval]. Dispatching only starts the
  run; `validate --import-run <run id>` records it when it finishes.

Decisions:
- The 75k gate's runs are the first L3 evidence: a shard there held the same
  load a shard holds at 1M (about 25,000 sessions), so per-shard points
  count and shared ones do not.
- Exactly-once (A1) from a cloud run is "0 errors and every turn
  completed"; the run has no tool ledger, so A2 and A3 come from L1/L2.
- D7 needs the driver pods' CPU limit; the workflow runs them without one,
  so D7 is not measured from these runs (noted, not guessed).
"""

import json
import math
import os
import re
import statistics
import subprocess

from . import Check, Output

PUT_USD = 0.005 / 1000


def read_run(path):
    def load(name, jsonl=False):
        f = os.path.join(path, name)
        if not os.path.exists(f):
            return [] if jsonl else None
        with open(f) as fh:
            return [json.loads(l) for l in fh if l.strip()] if jsonl else (
                json.load(fh) if name.endswith(".json") else fh.read())
    fin = (load("finished") or "").strip()
    return {"summary": load("summary.json"), "samples": load("samples.jsonl", True),
            "usage": load("usage.jsonl", True), "chaos": load("chaos.log") or "",
            "finished": int(fin) if fin.isdigit() else None}


def condense(samples):
    """Per-sample fleet figures: what the adapters need, a few KB per run."""
    rows = []
    for r in samples:
        nodes = []
        for sh, ns in (r.get("shards") or r.get("cells") or {}).items():  # "cells" before 2026-11-22
            for n, s in ns:
                if isinstance(s, dict):
                    nodes.append({"id": f"{sh}/{n}", "rss": s.get("rss_mb") or 0, "res": s.get("resident_mb") or 0,
                                  "log": s.get("log_mb") or 0, "sessions": s.get("sessions") or 0,
                                  "leader": s.get("leader") == s.get("node"), "unshipped_ms": s.get("unshipped_ms")})
        rows.append({"t": r["t"], "nodes": nodes})
    return rows


def unshipped_p99_s(rows):
    vals = sorted(n["unshipped_ms"] for r in rows for n in r["nodes"] if n["leader"] and n["unshipped_ms"] is not None)
    if not vals:
        return None
    return vals[min(len(vals) - 1, int(0.99 * len(vals)))] / 1000


def import_run(run, run_id, level, goal_agents):
    s = run["summary"]
    rows = condense(run["samples"])
    shards = len({n["id"].split("/")[0] for r in rows for n in r["nodes"]}) or 3
    peak_sessions = s.get("peak_sessions") or s.get("sessions")
    out = Output(agents=s.get("sessions"), density=peak_sessions / shards)
    ev = f"saas dataplane-load run {run_id}"
    errors = s.get("errors") or 0
    restarts = s.get("restarts") or {}
    ooms = sum(int(m.group(1)) for v in restarts.values() for m in [re.match(r"(\d+) \(OOMKilled\)", str(v))] if m)
    out.add("A1", level, errors == 0, note=f"{errors:,} errors over {s.get('turns_completed', 0):,} turns", evidence=ev)
    out.add("A4", level, ooms, note=", ".join(f"{k} {v}" for k, v in restarts.items()) or "no restarts", evidence=ev)
    out.add("E1", level, errors == 0 and bool(run["chaos"].strip()),
            note=(run["chaos"].strip().splitlines() or ["no chaos"])[0], evidence=ev)
    per_shard = peak_sessions / shards
    c1 = s["peak_node_rss_mb"] / per_shard * 1000
    out.add("C1", level, round(c1, 1), note=f"peak node {s['peak_node_rss_mb']:,} MB at {per_shard:,.0f} sessions per shard", evidence=ev)
    turns = s.get("turns_completed") or 0
    puts = sum((s.get("store_puts") or {}).values())
    if puts and turns:
        out.add("C5", level, round(puts / turns, 3), evidence=ev)
        out.metrics["store_puts_per_turn"] = round(puts / turns, 3)
    out.metrics["node_mb_per_1k_sessions"] = round(c1, 1)
    if rows:
        out.metrics["node_base_mb"] = round(statistics.fmean(n["rss"] for n in rows[0]["nodes"]) or 60)
        # C3: what the node does not account for, at each node's peak.
        worst = 0.0
        for nid in {n["id"] for r in rows for n in r["nodes"]}:
            pk = max((n for r in rows for n in r["nodes"] if n["id"] == nid), key=lambda n: n["rss"])
            if pk["rss"] > 1000:
                worst = max(worst, (pk["rss"] - pk["res"] - pk["log"]) / pk["rss"])
        out.add("C3", level, round(worst, 3), note="peak node: resident set less resident events and Raft log", evidence=ev)
        p99 = unshipped_p99_s(rows)
        if p99 is not None:
            out.add("F3", level, round(p99, 2), note="leaders' oldest unshipped entry, p99 over the run", evidence=ev)
    usage = (s.get("usage") or {}).get("components") or {}
    tps = s.get("turns_per_sec") or 0
    if usage and tps:
        cell, exe = usage.get("cell", {}), usage.get("executor", {})
        shard_ms = cell["cpu_cores_avg"] * 1000 / tps
        out.add("C4", level, round(shard_ms, 1), note=f"{cell['cpu_cores_avg']} cell cores at {tps} turns/s", evidence=ev)
        out.metrics["shard_core_ms_per_turn"] = round(shard_ms, 1)
        out.metrics["executor_core_ms_per_turn"] = round(exe["cpu_cores_avg"] * 1000 / tps, 1)
        if s.get("peak_running_turns"):
            c2 = exe["memory_mb_peak"] / s["peak_running_turns"]
            out.add("C2", level, round(c2, 3), note=f"{exe['memory_mb_peak']:,} MB at {s['peak_running_turns']:,} running turns", evidence=ev)
            out.metrics["executor_mb_per_running_turn"] = round(c2, 3)
        backlog = [b["running"] for b in s.get("backlog", []) if b["running"]]
        if backlog:
            out.metrics["turn_seconds_in_flight"] = round(statistics.fmean(backlog) / tps, 1)
        if s.get("compute_usd_estimate") is not None and rows:
            hours = rows[-1]["t"] / 3600
            agent_hours = s["sessions"] * hours
            usd = (s["compute_usd_estimate"] + puts * PUT_USD) / agent_hours
            out.add("C6", level, round(usd, 7), note=f"${s['compute_usd_estimate']} compute + ${puts * PUT_USD:.2f} store writes over {agent_hours:,.0f} agent-hours", evidence=ev)
            out.metrics["usd_per_agent_hour"] = round(usd, 7)
    out.notes.append("D7 not measured: the workflow runs drivers without a CPU limit")
    out.notes.append("F1 not measured: in the 75k load every session stops after 10 turns, so memory "
                     "falls after the first half hour and there is no steady hour to measure creep on")
    out.metrics["run_summary"] = {k: s.get(k) for k in ("sessions", "turns_completed", "errors", "shed", "turns_per_sec",
                                                         "peak_sessions", "peak_running_turns", "peak_node_rss_mb")}
    return out


def fetch(run_id, dest):
    """Download a saas workflow run's artifact into dest (needs gh)."""
    os.makedirs(dest, exist_ok=True)
    r = subprocess.run(["gh", "api", f"repos/everruns/saas/actions/runs/{run_id}/artifacts"],
                       capture_output=True, text=True, check=True)
    arts = json.loads(r.stdout)["artifacts"]
    if not arts:
        raise SystemExit(f"run {run_id} has no artifact")
    zpath = os.path.join(dest, "artifact.zip")
    with open(zpath, "wb") as f:
        subprocess.run(["gh", "api", arts[0]["archive_download_url"]], stdout=f, check=True)
    subprocess.run(["unzip", "-o", "-q", zpath, "-d", dest], check=True)
    return dest


def dispatch_inputs(cfg, agents):
    lay, g = cfg.layout, cfg.goal
    shards = math.ceil(agents / g["density"]["sessions_per_shard"])
    turns = 10
    return {
        "sessions": str(agents),
        "ramp": str(g["pace"]["ramp_minutes"] * 60),
        "turns": str(turns),
        "think": str(max(1, g["pace"]["turn_every_s"] - 100)),  # ~100 s turns in flight at 75k
        "stream": str(g["agents"]["streaming_share"]),
        "cells": str(shards),
        "executors": str(max(8, math.ceil(8 * agents / 75_000))),
        "drivers": str(max(3, math.ceil(3 * agents / 75_000))),
        "cell_memory": f"{lay['node_memory_limit_mb'] // 1024}Gi",
        "executor_memory": f"{lay['executor_memory_limit_mb'] // 1024}Gi",
        "agent": "mix",
        "chaos": "true",
    }


def dispatch(cfg, level, spend, dry_run=False):
    if not spend or not spend.strip():
        raise SystemExit(f"{level} costs money: pass --spend \"why this run, now\"")
    if level == "L4" and not cfg.goal.get("approval", {}).get("full_run"):
        raise SystemExit("the full run needs the owner's approval recorded in goal.toml [approval] full_run")
    agents = cfg.goal["agents"]["total"] if level == "L4" else max(125_000, cfg.goal["agents"]["total"] // 8)
    inputs = dispatch_inputs(cfg, agents)
    cmd = ["gh", "api", "-X", "POST", "repos/everruns/saas/actions/workflows/dataplane-load.yml/dispatches",
           "-f", "ref=main"] + [x for k, v in inputs.items() for x in ("-f", f"inputs[{k}]={v}")]
    if dry_run:
        return cmd
    subprocess.run(cmd, check=True)
    return cmd


CHECKS = [
    Check("cloud-slice", "L3", ["A1", "A4", "C1", "C2", "C3", "C4", "C5", "C6", "E1", "F3"],
          minutes=90, cost_usd=20, needs=("gh",),
          command='validate --level cloud-slice --spend "<reason>", then validate --import-run <run id>'),
    Check("full", "L4", ["A1", "A4", "C1", "C2", "C3", "C4", "C5", "C6", "E1", "F3", "R10"],
          minutes=150, cost_usd=200, needs=("gh",),
          command='validate --level full --spend "<reason>" (owner approval in goal.toml)'),
]
