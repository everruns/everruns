"""What this verifier adds to the generic engine (lib/engine/hooks.py):
agents, shards and pace as target names, the density and run-size rules,
proven agents, EKS run import and paid dispatch, and the report sections
about resources and the coding profile.

Kept per verifier on purpose (owner, 2026-11-24): agents-1m and agents-75k
verify different goals and may diverge, so each owns its copy.
"""

import datetime as dt
import math
import os

import checks as K
from engine import score as S
from engine.hooks import Hooks
from engine.report import fmt, latest_metric


class AgentsVerifier(Hooks):
    size_unit = "agents"
    dirty_paths = ("crates", "scripts", "verifiers")
    default_levels = ("L0", "L1")    # model + simulation: free and automatic

    # Goal and targets
    def default_size(self, cfg):
        return cfg.goal["agents"]["total"]

    def milestone_sizes(self, cfg):
        return cfg.goal.get("milestones", {}).get("agents", [])

    def full_size(self, cfg, agents=None):
        """Agents a run needs for shared points: the goal's (or milestone's)
        count times [agents] full_run_min_share (1.0 unless the goal says)."""
        n = agents or cfg.default_size()
        return n * cfg.goal["agents"].get("full_run_min_share", 1.0)

    def derived(self, cfg, n):
        g = cfg.goal
        pace = g["pace"]
        return {
            "agents": n,
            "coding_agents": n * g["agents"]["coding_share"],
            "turns_per_sec": n / pace["turn_every_s"],
            "peak_turns_per_sec": n / pace["turn_every_s"] * pace["peak_multiplier"],
            "ramp_per_sec": n / (pace["ramp_minutes"] * 60),
            "shards": math.ceil(n / g["density"]["sessions_per_shard"]),
        }

    # Checks
    def checks(self):
        return K.all_checks()

    def not_built(self):
        return K.NOT_BUILT

    def context(self, cfg, args):
        from checks import model
        return {"profile": model.load_profile(), "size": args.size}

    def add_args(self, ap):
        ap.add_argument("--size", type=float, default=1.0, help="local runs at this share of goal density")
        ap.add_argument("--spend", help="reason for a paid cloud run (needed by cloud-slice and full)")
        ap.add_argument("--dry-run", action="store_true", help="print the paid dispatch, start nothing")
        ap.add_argument("--import-run", metavar="RUN", help="score a finished saas dataplane-load run (id or dir)")
        ap.add_argument("--commit", help="everruns commit an imported run's image was built from")

    # Runs and scoring
    def stamp(self, outputs, run):
        agents, coding = None, False
        for out in outputs:
            for e in out.entries:
                if getattr(out, "density", None) is not None:
                    e.setdefault("density", out.density)
                if getattr(out, "agents", None) is not None:
                    e.setdefault("agents", out.agents)
            agents = agents or getattr(out, "agents", None)
            coding = coding or getattr(out, "coding_load", False)
        run["agents"] = agents
        run["coding_load"] = coding

    def exclusion(self, cfg, run, point, entry, size):
        lvl = int(entry["level"][1:])
        density = entry.get("density", run.get("density"))
        want = cfg.goal["density"]["sessions_per_shard"]
        if point.scales == "per_shard" and lvl >= 2 and (density is None or density < 0.95 * want):
            return f"below goal density ({density or 0:,.0f} < {want:,} sessions per shard)"
        n = entry.get("agents", run.get("agents")) or 0
        if point.scales == "shared" and lvl >= 3 and n < self.full_size(cfg, size):
            return f"shared part from a run of {n:,} agents, below {size:,}"
        return None

    def command(self, cfg, a, env):
        from checks import cloud
        level = env["level"]
        if level in ("L3", "L4"):
            cmd = cloud.dispatch(cfg, level, a.spend, dry_run=a.dry_run)
            print(("would run: " if a.dry_run else "started: ") + " ".join(cmd))
            print("when it finishes: verify --import-run <run id> --record")
            return 0
        if not a.import_run:
            return None
        path = a.import_run
        if not os.path.isdir(path):
            path = cloud.fetch(a.import_run, os.path.join(env["evidence"], "artifact"))
        run_id = os.path.basename(os.path.normpath(a.import_run))
        data = cloud.read_run(path)
        agents = (data["summary"] or {}).get("sessions") or 0
        lvl = level or ("L4" if agents >= self.full_size(cfg) or "L3" not in cfg.levels else "L3")
        out = cloud.import_run(data, run_id, lvl, cfg.goal["agents"]["total"])
        extra = {"imported": run_id}
        if data.get("finished"):  # the run's own time, so "newest result wins" holds
            extra["at"] = dt.datetime.fromtimestamp(data["finished"], dt.timezone.utc).isoformat().replace("+00:00", "Z")
        run = env["make_run"](cfg, [out], [lvl], f"import: saas dataplane-load run {run_id}", env["started"],
                              commit=a.commit or "", extra=extra)
        run["run"] = f"{run['at']}-{lvl}-saas-{run_id}"
        return run

    # Report
    def proven_agents(self, cfg, runs):
        """Largest L3/L4 run whose gate results all passed (and had at least one)."""
        best = 0
        gates = {p.id for p in cfg.points if p.gate}
        for run in runs:
            if run.get("fingerprint") != cfg.fingerprint or run.get("invalid"):
                continue
            es = [e for e in run.get("entries", []) if e["point"] in gates and e["level"] in ("L3", "L4")]
            if es and all(S.passes(cfg.point(e["point"]), e, cfg.target(cfg.point(e["point"]))) for e in es):
                best = max(best, run.get("agents") or 0)
        return best

    def summary(self, cfg, runs, fresh):
        return {
            "proven_agents": self.proven_agents(cfg, runs),
            "projected_agents": latest_metric(runs, "projected_agents")[0],
            "next_limit": latest_metric(runs, "next_limit")[0],
            "resources": latest_metric(runs, "resources")[0],
            "model_inputs": latest_metric(runs, "model_inputs")[0],
            "assumptions": latest_metric(runs, "model_assumptions")[0],
            "coding_profile": latest_metric(runs, "coding_profile")[0],
        }

    def record_fields(self):
        return ("proven_agents", "projected_agents")

    def history_columns(self):
        return [("Proven", "proven_agents"), ("Projected", "projected_agents")]

    def headline(self, cfg, rep):
        s = f"Proven agents {fmt(rep['proven_agents'])}"
        if cfg.has("D1"):
            proj = rep["projected_agents"]
            s += f"; projected agents {fmt(proj) if proj else 'unknown until D1 (scale.py) runs'}"
        return s + "."

    def sections(self, cfg, rep):
        return ["\n## Next limit\n",
                rep["next_limit"] or "None crossed in the cluster simulation, or it has not run yet."]

    def after_milestones(self, cfg, rep):
        r = rep.get("resources")
        if not r:
            return []
        L = ["\n## Resources for the goal (cluster simulation)\n",
             f"{fmt(r['agents'])} agents: {r['shards']} shards ({r['node_pods']} node pods) and "
             f"{r['executor_pods']} executors on {', '.join(f'{v} x {k}' for k, v in r['machines'].items())}: "
             f"{fmt(r['vcpus'])} vCPU, {fmt(r['memory_gb'])} GB, ${r['machines_usd_per_hour']:,.2f}/h machines "
             f"+ ${r['store_usd_per_hour']:,.2f}/h object store writes.\n",
             "| Pod | Peak | After a zone loss |\n|---|---|---|"]
        for k, v in r["worst"].items():
            L.append(f"| {k} | {v['peak']:.0%} | {v['zone_loss']:.0%} |")
        return L

    def notes(self, cfg, rep):
        L = []
        for k, v in (rep.get("model_inputs") or {}).items():
            L.append(f"- measured {k} = {fmt(v['value'])} ({v['level']}, {v['from']})")
        for k, v in (rep.get("assumptions") or {}).items():
            L.append(f"- assumed {k} = {fmt(v)} (until agents-75k measures it, G1/G2)")
        cp = rep.get("coding_profile")
        if cp:
            L.append(f"- coding profile: {cp['sessions']} sessions, {cp['turns']} turns, captured {cp['captured']}"
                     + (" (few sessions: refresh with workload/capture.py on more transcripts)" if (cp["sessions"] or 0) < 20 else ""))
        return L


VERIFIER = AgentsVerifier()
