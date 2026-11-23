"""validate: the single entry point of the 1M agents validator.

  validate                         model + simulation levels, then the report (free)
  validate --quick                 the same without the cargo fleet tests (seconds)
  validate --plan                  the climb list only; runs nothing
  validate --score                 recompute the score from results.jsonl
  validate --level local [--points C1,F1] [--size 1.0]
  validate --level cloud-slice --spend "reason"     paid; starts the SaaS workflow
  validate --level full --spend "reason"            paid; needs the owner's approval
  validate --import-run <run id | dir> [--commit SHA] [--level L3]
  validate --history               the trend from results.jsonl and attempts.jsonl
  validate --checkpoint NAME       freeze the current per-point state
  validate --against NAME          compare with a checkpoint; exit 1 on a regression
  validate --attempt "idea" --points C1 --kept no --why "..."   journal a climbing attempt
  validate --compact               fold runs older than 90 days

Add --record to append the run to results.jsonl (commit it with the change
it measured). Reports go to target/validators/agents-1m/<time>/.
"""

import argparse
import datetime as dt
import json
import os
import platform
import sys
import time
from dataclasses import dataclass

from . import config as C
from . import report as R
from . import score as S
from . import store as T

LEVEL_NAMES = {"model": "L0", "simulation": "L1", "local": "L2", "cloud-slice": "L3", "full": "L4"}


@dataclass
class Ctx:
    cfg: object
    runs: list
    profile: dict
    evidence: str
    size: float = 1.0


def now_iso():
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def load_profile():
    from checks import model
    return model.load_profile()


def make_run(cfg, outputs, levels, source, started, commit=None, extra=None):
    entries, metrics, notes = [], {}, []
    coding = False
    agents = None
    for out in outputs:
        for e in out.entries:
            if out.density is not None:
                e.setdefault("density", out.density)
            if out.agents is not None:
                e.setdefault("agents", out.agents)
            entries.append(e)
        metrics.update(out.metrics)
        notes += out.notes
        coding = coding or out.coding_load
        agents = agents or out.agents
    run = {
        "run": f"{now_iso()}-{''.join(sorted(set(levels)))}",
        "at": now_iso(), "commit": commit if commit is not None else T.head_commit(),
        "dirty": T.dirty(["crates", "scripts", "validators"]) if commit is None else False,
        "fingerprint": cfg.fingerprint, "levels": sorted(set(levels)), "source": source,
        "machine": f"{platform.system()} {platform.machine()} {os.cpu_count()} cpu",
        "duration_s": round(time.time() - started), "cost_usd": sum(o.cost_usd for o in outputs),
        "agents": agents, "coding_load": coding,
        "entries": entries, "metrics": metrics, "notes": notes,
    }
    run.update(extra or {})
    run["invalid"] = S.run_validity(cfg, run)
    # What each entry's evidence depends on, as a digest of the watched files.
    digests = {}
    for e in entries:
        p = cfg.point(e["point"])
        key = tuple(p.watches)
        if key not in digests:
            if commit is None:       # this checkout
                digests[key] = T.watch_hash(p.watches)
            elif commit:             # an imported run built from a known commit
                digests[key] = T.watch_hash(p.watches, commit)
            else:                    # unknown code: freshness halves
                digests[key] = None
        e["watch"] = digests[key]
    return run


def freshness():
    return S.Freshness(changed=T.changed_since, current=T.watch_hash)


def write_reports(rep, cfg, evidence):
    os.makedirs(evidence, exist_ok=True)
    md = R.markdown(rep, cfg)
    with open(os.path.join(evidence, "report.md"), "w") as f:
        f.write(md)
    with open(os.path.join(evidence, "report.json"), "w") as f:
        f.write(R.to_json(rep))
    return md


def record(cfg, run, rep):
    run["recorded"] = True
    run["score"] = {"score": rep["score"], "uncapped": rep["uncapped"], "failed_gates": rep["failed_gates"],
                    "proven_agents": rep["proven_agents"], "projected_agents": rep["projected_agents"]}
    # The timeline is evidence, not history: keep results.jsonl small.
    run["metrics"] = {k: v for k, v in run["metrics"].items() if k != "cluster_timeline"}
    T.append_jsonl(T.RESULTS, run)


def run_checks(ctx, checks, points=None):
    outs = []
    for ch in checks:
        if points and not set(points) & set(ch.points):
            continue
        ok, why = ch.available()
        if not ok:
            print(f"skip {ch.name}: {why}", file=sys.stderr)
            continue
        print(f"run {ch.name} ({ch.level})", file=sys.stderr)
        outs.append(ch.run(ctx))
    return outs


def cmd_history(cfg):
    rows = [r for r in T.read_jsonl(T.RESULTS)]
    print("| When | Levels | Score | Proven | Projected | Source | Commit |\n|---|---|---|---|---|---|---|")
    for r in rows:
        sc = r.get("score") or {}
        print(f"| {r['at'][:16]} | {','.join(r.get('levels', []))} | {sc.get('score', '-')} | "
              f"{R.fmt(sc.get('proven_agents'))} | {R.fmt(sc.get('projected_agents'))} | {r.get('source')} | {(r.get('commit') or '-')[:10]} |")
    att = T.read_jsonl(T.ATTEMPTS)
    if att:
        print("\n| When | Idea | Points | Before | After | Kept | Why |\n|---|---|---|---|---|---|---|")
        for a in att:
            print(f"| {a['at'][:16]} | {a['idea']} | {','.join(a['points'])} | {a.get('before', '-')} | "
                  f"{a.get('after', '-')} | {a['kept']} | {a.get('why', '')} |")


def snapshot(rep):
    return {"at": now_iso(), "score": rep["score"], "fingerprint": rep["fingerprint"],
            "points": {p["id"]: {"value": p["value"], "m": p["m"], "level": p["best_level"], "credit": p["credit"]}
                       for p in rep["points"]}}


def compare(cp, rep, cfg):
    """Points whose closeness or credit fell, and the score change."""
    worse = []
    for p in rep["points"]:
        was = cp["points"].get(p["id"])
        if not was or was["m"] is None:
            continue
        if p["m"] is None or p["m"] < was["m"] - 0.02 or p["credit"] < was["credit"] - 0.02:
            worse.append(f"{p['id']}: m {was['m']} -> {p['m']}, credit {was['credit']} -> {p['credit']}")
    return worse, round(rep["score"] - cp["score"], 2)


def main(argv=None):
    ap = argparse.ArgumentParser(prog="validate", description="1M agents validator",
                                 formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__)
    ap.add_argument("--level", choices=list(LEVEL_NAMES))
    ap.add_argument("--points", help="comma-separated point ids")
    ap.add_argument("--size", type=float, default=1.0, help="local runs at this share of goal density")
    ap.add_argument("--quick", action="store_true", help="skip checks over 5 minutes")
    ap.add_argument("--plan", action="store_true")
    ap.add_argument("--score", action="store_true")
    ap.add_argument("--json", action="store_true", help="print report.json instead of report.md")
    ap.add_argument("--record", action="store_true")
    ap.add_argument("--spend")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--import-run", metavar="RUN")
    ap.add_argument("--commit", help="everruns commit an imported run's image was built from")
    ap.add_argument("--history", action="store_true")
    ap.add_argument("--checkpoint")
    ap.add_argument("--against")
    ap.add_argument("--attempt")
    ap.add_argument("--kept", choices=["yes", "no"])
    ap.add_argument("--before", type=float)
    ap.add_argument("--after", type=float)
    ap.add_argument("--why")
    ap.add_argument("--compact", action="store_true")
    ap.add_argument("--out", help="evidence directory (default target/validators/agents-1m/<time>)")
    a = ap.parse_args(argv)

    cfg = C.load()
    runs = T.read_jsonl(T.RESULTS)
    from checks import registry
    checks = registry()
    started = time.time()
    evidence = a.out or os.path.join(T.EVIDENCE, now_iso().replace(":", ""))
    points = [p.strip() for p in a.points.split(",")] if a.points else None

    if a.history:
        cmd_history(cfg)
        return 0
    if a.compact:
        T.write_jsonl(T.RESULTS, T.compact(runs))
        return 0
    if a.attempt:
        if not (a.points and a.kept):
            ap.error("--attempt needs --points and --kept")
        T.append_jsonl(T.ATTEMPTS, {"at": now_iso(), "idea": a.attempt, "points": points, "before": a.before,
                                    "after": a.after, "kept": a.kept, "why": a.why, "commit": T.head_commit()})
        return 0

    current = None
    if a.level in ("cloud-slice", "full"):
        from checks import cloud
        lvl = LEVEL_NAMES[a.level]
        cmd = cloud.dispatch(cfg, lvl, a.spend, dry_run=a.dry_run)
        print(("would run: " if a.dry_run else "started: ") + " ".join(cmd))
        print("when it finishes: validate --import-run <run id> --record")
        return 0
    if a.import_run:
        from checks import cloud
        path = a.import_run
        if not os.path.isdir(path):
            path = cloud.fetch(a.import_run, os.path.join(evidence, "artifact"))
        run_id = os.path.basename(os.path.normpath(a.import_run))
        data = cloud.read_run(path)
        agents = (data["summary"] or {}).get("sessions") or 0
        lvl = LEVEL_NAMES.get(a.level) or ("L4" if agents >= cfg.goal["agents"]["total"] else "L3")
        out = cloud.import_run(data, run_id, lvl, cfg.goal["agents"]["total"])
        extra = {"imported": run_id}
        if data.get("finished"):  # the run's own time, so "newest result wins" holds
            extra["at"] = dt.datetime.fromtimestamp(data["finished"], dt.timezone.utc).isoformat().replace("+00:00", "Z")
        current = make_run(cfg, [out], [lvl], f"import: saas dataplane-load run {run_id}", started,
                           commit=a.commit or "", extra=extra)
        current["run"] = f"{current['at']}-{lvl}-saas-{run_id}"
    elif not (a.plan or a.score or a.checkpoint or a.against):
        ctx = Ctx(cfg, runs, load_profile(), evidence, a.size)
        os.makedirs(evidence, exist_ok=True)
        if a.level == "local":
            chosen = [c for c in checks if c.level == "L2"]
            levels = ["L2"]
        else:
            chosen = [c for c in checks if c.level in ("L0", "L1") and not (a.quick and c.minutes > 5)]
            # Simulation checks first, so the model sees nothing of its own.
            chosen.sort(key=lambda c: c.level, reverse=True)
            levels = ["L0", "L1"]
        outs = run_checks(ctx, chosen, points)
        current = make_run(cfg, outs, levels, "validate" + (f" --level {a.level}" if a.level else ""), started)

    allruns = runs + ([current] if current else [])
    fresh = freshness()
    rep = R.build(cfg, allruns, fresh, checks, current)
    if current:
        rep["notes"] = current["notes"] + (["run invalid: " + "; ".join(current["invalid"])] if current["invalid"] else [])
        if current.get("dirty"):
            rep["notes"].append("uncommitted changes: commit them with this result so freshness can track it")
    if a.plan:
        for i, x in enumerate(rep["climb"], 1):
            print(f"{i:2}. +{x['gain']:<6} {x['hours']:>5.1f} h  {x['what']}" + (f"\n      {x['command']}" if x["command"] else ""))
        if rep["paid"]:
            print("\npaid (need --spend):")
            for x in rep["paid"]:
                print(f"    +{x['gain']:<6} ~${x['usd']}  {x['what']}")
        return 0
    if a.checkpoint:
        os.makedirs(T.CHECKPOINTS, exist_ok=True)
        with open(T.checkpoint_path(a.checkpoint), "w") as f:
            json.dump(snapshot(rep), f, indent=1, sort_keys=True)
            f.write("\n")
        print(f"checkpoint {a.checkpoint}: score {rep['score']}")
        return 0
    md = write_reports(rep, cfg, evidence)
    if a.record and current:
        record(cfg, current, rep)
        print(f"recorded {current['run']} in results.jsonl", file=sys.stderr)
    if a.against:
        with open(T.checkpoint_path(a.against)) as f:
            cp = json.load(f)
        worse, delta = compare(cp, rep, cfg)
        print(f"against {a.against}: score {delta:+}")
        for w in worse:
            print("  worse " + w)
        return 1 if worse else 0
    sys.stdout.write(R.to_json(rep) if a.json else md)
    print(f"\nreport: {os.path.join(evidence, 'report.md')}", file=sys.stderr)
    return 0
