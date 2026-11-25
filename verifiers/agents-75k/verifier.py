"""What this verifier adds to the generic engine (lib/engine/hooks.py):
agents, shards and pace as target names, the density and run-size rules,
proven agents, EKS run import and paid dispatch, and the report notes
about the model's inputs and the coding profile.

Kept per verifier on purpose (owner, 2026-11-24): agents-1m and agents-75k
verify different goals and may diverge, so each owns its copy.
"""

import datetime as dt
import math
import os
import re

import checks as K
from engine import score as S
from engine.cli import gate_streak
from engine.hooks import Hooks
from engine.report import fmt, latest_metric


def imported_ids(runs):
    """saas run ids already in the journal."""
    ids = set()
    for r in runs:
        rid = re.sub(r"\D", "", str(r.get("imported") or ""))
        if rid:
            ids.add(int(rid))
    return ids


def next_gate_run(cfg, env):
    """The oldest finished gate run newer than every imported one: R10
    counts passing runs in a row, so runs go into the journal in the order
    they happened. Smaller runs on the way are passed over."""
    from checks import cloud
    done = imported_ids(env["runs"])
    for rid in cloud.gate_runs(max(done, default=0)):
        path = cloud.fetch(rid, os.path.join(env["evidence"], f"artifact-{rid}"))
        data = cloud.read_run(path)
        if cloud.is_gate(data, cfg.goal["agents"]["total"]):
            return str(rid), path, data
        print(f"run {rid}: not at the gate's size, passed over")
    return None


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
        ap.add_argument("--commit", help="everruns commit an imported run's image was built from "
                        "(default: the revision label of the agentd image the run deployed)")
        ap.add_argument("--run-id", help="the saas run id of an --import-run directory (default: its digits)")
        ap.add_argument("--import-new", action="store_true",
                        help="import the oldest gate run not yet recorded (repeat until none is left)")

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

    # Scaled shard soaks (levels.toml [L2.scaled]): below goal density a soak
    # counts when its Raft log reached about the gate's share of the limit,
    # at a certainty scaled by how often a soak's out-of-memory verdict
    # matched the next imported gate run's.
    _gate_share = None
    _agreement = (0, 0)

    def _scaled(self, cfg):
        return cfg.levels.get("L2", {}).get("scaled")

    def _below_density(self, cfg, run, entry):
        density = entry.get("density", run.get("density"))
        return density is None or density < 0.95 * cfg.goal["density"]["sessions_per_shard"]

    def prepare(self, cfg, runs):
        sc = self._scaled(cfg)
        if not sc:
            return
        gates, soaks = [], []
        for run in sorted(runs, key=lambda r: r["at"]):
            for e in run.get("entries", []):
                if e["point"] != "A4":
                    continue
                if e["level"] == "L4":
                    gates.append((run["at"], e))
                elif e["level"] == "L2" and "log_share" in e and self._below_density(cfg, run, e):
                    soaks.append((run["at"], e))
        self._gate_share = sc["gate_log_share"]
        for _, e in reversed(gates):
            if e.get("peak_log_mb"):
                self._gate_share = e["peak_log_mb"] / sc["gate_limit_mb"]
                break
        agree = n = 0
        for at, g in gates:
            before = [e for t, e in soaks if t < at]
            if before:
                n += 1
                agree += (before[-1]["value"] == 0) == (g["value"] == 0)
        self._agreement = (agree, n)

    def _calibrated(self, cfg, entry):
        sc = self._scaled(cfg)
        share, gate = entry.get("log_share"), self._gate_share
        if not sc or share is None or not gate or entry["level"] != "L2":
            return False
        return abs(share - gate) <= sc["log_share_tolerance"] * gate

    def certainty(self, cfg, run, point, entry):
        c = cfg.certainty(entry["level"])
        if point.scales == "per_shard" and entry["level"] == "L2" and self._below_density(cfg, run, entry):
            agree, n = self._agreement
            c = self._scaled(cfg)["certainty"] * (agree + 1) / (n + 1)
        return c

    def exclusion(self, cfg, run, point, entry, size):
        lvl = int(entry["level"][1:])
        density = entry.get("density", run.get("density"))
        want = cfg.goal["density"]["sessions_per_shard"]
        if point.scales == "per_shard" and lvl >= 2 and (density is None or density < 0.95 * want):
            if not self._calibrated(cfg, entry):
                share = entry.get("log_share")
                why = (f"; log at {share:.1%} of its limit, the gate's {self._gate_share:.1%}"
                       if share is not None and self._gate_share else "")
                return f"below goal density ({density or 0:,.0f} < {want:,} sessions per shard){why}"
        n = entry.get("agents", run.get("agents")) or 0
        if point.scales == "shared" and lvl >= 3 and n < self.full_size(cfg, size):
            return f"shared part from a run of {n:,} agents, below {size:,}"
        return None

    def skip_validity(self, cfg, run, point):
        # The coding realism check (G6) only judges runs that played the coding load.
        return point.id == "G6" and not run.get("coding_load", True)

    def command(self, cfg, a, env):
        from checks import cloud
        level = env["level"]
        if level in ("L3", "L4"):
            cmd = cloud.dispatch(cfg, level, a.spend, dry_run=a.dry_run)
            print(("would run: " if a.dry_run else "started: ") + " ".join(cmd))
            print("when it finishes: verify --import-run <run id> --record")
            return 0
        if a.import_new:
            found = next_gate_run(cfg, env)
            if found is None:
                print("no gate run left to import")
                return 0
            run_id, path, data = found
        elif a.import_run:
            path = a.import_run
            if not os.path.isdir(path):
                path = cloud.fetch(a.import_run, os.path.join(env["evidence"], "artifact"))
            run_id = a.run_id or re.sub(r"\D", "", os.path.basename(os.path.normpath(a.import_run))) or a.import_run
            data = cloud.read_run(path)
        else:
            return None
        commit = a.commit
        if commit is None:
            digest = cloud.run_image(path)
            commit = cloud.image_commit(digest) if digest else None
            if commit is None:
                print("warning: no agentd image commit found; freshness will treat this run as old code")
        agents = (data["summary"] or {}).get("sessions") or 0
        lvl = level or ("L4" if agents >= self.full_size(cfg) or "L3" not in cfg.levels else "L3")
        out = cloud.import_run(data, run_id, lvl, cfg.goal["agents"]["total"])
        if cfg.has("R10"):
            streak = gate_streak(cfg, env["runs"], out.entries)
            out.add("R10", lvl, streak, note=f"{streak} imported runs in a row pass every gate, this one included")
        extra = {"imported": run_id}
        if data.get("finished"):  # the run's own time, so "newest result wins" holds
            extra["at"] = dt.datetime.fromtimestamp(data["finished"], dt.timezone.utc).isoformat().replace("+00:00", "Z")
        run = env["make_run"](cfg, [out], [lvl], f"import: saas dataplane-load run {run_id}", env["started"],
                              commit=commit or "", extra=extra)
        run["run"] = f"{run['at']}-{lvl}-saas-{run_id}"
        return run

    # Report
    def proven_agents(self, cfg, runs):
        """Largest L3/L4 run whose gate results all passed (and had at least one)."""
        best = 0
        gates = {p.id for p in cfg.points if p.gate}
        for run in runs:
            if not cfg.accepts(run.get("fingerprint")) or run.get("invalid"):
                continue
            es = [e for e in run.get("entries", []) if e["point"] in gates and e["level"] in ("L3", "L4")]
            if es and all(S.passes(cfg.point(e["point"]), e, cfg.target(cfg.point(e["point"]))) for e in es):
                best = max(best, run.get("agents") or 0)
        return best

    def summary(self, cfg, runs, fresh):
        return {
            "proven_agents": self.proven_agents(cfg, runs),
            "model_inputs": latest_metric(runs, "model_inputs")[0],
            "assumptions": latest_metric(runs, "model_assumptions")[0],
            "realism": latest_metric(runs, "realism")[0],
            "coding_profile": latest_metric(runs, "coding_profile")[0],
        }

    def record_fields(self):
        return ("proven_agents",)

    def history_columns(self):
        return [("Proven", "proven_agents")]

    def headline(self, cfg, rep):
        return f"Proven agents {fmt(rep['proven_agents'])}."

    def notes(self, cfg, rep):
        L = []
        for k, v in (rep.get("model_inputs") or {}).items():
            L.append(f"- measured {k} = {fmt(v['value'])} ({v['level']}, {v['from']})")
        for k, v in (rep.get("assumptions") or {}).items():
            L.append(f"- assumed {k} = {fmt(v)} (until G1/G2 measure it)")
        cp = rep.get("coding_profile")
        if cp:
            L.append(f"- coding profile: {cp['sessions']} sessions, {cp['turns']} turns, captured {cp['captured']}"
                     + (" (few sessions: refresh with workload/capture.py on more transcripts)" if (cp["sessions"] or 0) < 20 else ""))
        return L


VERIFIER = AgentsVerifier()
