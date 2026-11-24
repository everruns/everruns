"""verify: the single entry point of a verifier (verifiers/<name>/verify).

  verify                           the default levels' checks, then the report
  verify --quick                   the same without checks over 5 minutes
  verify --plan                    the climb list only; runs nothing
  verify --score                   recompute the score from results.jsonl
  verify --level NAME [--points C1,F1]   one level's checks (names in levels.toml)
  verify --history                 the trend from results.jsonl and attempts.jsonl
  verify --checkpoint NAME         freeze the current per-point state
  verify --against NAME            compare with a checkpoint; exit 1 on a regression
  verify --attempt "idea" --points C1 --kept no --why "..."     journal a climbing attempt
  verify --compact                 fold runs older than 90 days

A verifier adds its own options (listed above). Add --record to append the
run to results.jsonl (commit it with the change it measured). Reports go to target/verifiers/<name>/<time>/.
"""

import argparse
import datetime as dt
import json
import os
import platform
import sys
import time
from . import checks as K
from . import config as C
from . import report as R
from . import score as S
from . import store as T



class Ctx:
    """What a check receives: cfg, runs, evidence, plus the verifier's context."""

    def __init__(self, cfg, runs, evidence, **extra):
        self.cfg, self.runs, self.evidence = cfg, runs, evidence
        self.__dict__.update(extra)


def level_names(cfg):
    return {v["name"]: k for k, v in cfg.levels.items()}


def now_iso():
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def gate_streak(cfg, runs, entries):
    """Imported runs in a row, newest first and this one included, whose
    gate results all pass: evidence that a gate passes repeatedly, not once."""
    gates = [p for p in cfg.points if p.gate]

    def clean(es):
        mine = [e for e in es if any(e["point"] == g.id for g in gates)]
        return bool(mine) and all(S.passes(cfg.point(e["point"]), e, cfg.target(cfg.point(e["point"]))) for e in mine)

    if not clean(entries):
        return 0
    streak = 1
    for r in sorted((r for r in runs if r.get("imported") and r.get("fingerprint") == cfg.fingerprint),
                    key=lambda r: r["at"], reverse=True):
        if not clean(r["entries"]):
            break
        streak += 1
    return streak


def make_run(cfg, outputs, levels, source, started, commit=None, extra=None):
    entries, metrics, notes = [], {}, []
    for out in outputs:
        out.entries = [e for e in out.entries if cfg.has(e["point"])]  # a shared check can cover points this verifier lacks
        entries += out.entries
        metrics.update(out.metrics)
        notes += out.notes
    run = {
        "run": f"{now_iso()}-{''.join(sorted(set(levels)))}",
        "at": now_iso(), "commit": commit if commit is not None else T.head_commit(),
        "dirty": T.dirty(list(cfg.hooks.dirty_paths)) if commit is None else False,
        "fingerprint": cfg.fingerprint, "levels": sorted(set(levels)), "source": source,
        "machine": f"{platform.system()} {platform.machine()} {os.cpu_count()} cpu",
        "duration_s": round(time.time() - started), "cost_usd": sum(o.cost_usd for o in outputs),
        "entries": entries, "metrics": metrics, "notes": notes,
    }
    cfg.hooks.stamp(outputs, run)
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
    run["score"] = {"score": rep["score"], "uncapped": rep["uncapped"], "failed_gates": rep["failed_gates"]}
    run["score"].update({k: rep.get(k) for k in cfg.hooks.record_fields()})
    # Bulky evidence (metrics whose names end in _timeline) stays out of results.jsonl.
    run["metrics"] = {k: v for k, v in run["metrics"].items() if not k.endswith("_timeline")}
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
        # Decision: a check that breaks (a harness that printed no result, a
        # missing binary) is reported and skipped; the other checks of the run
        # still run and are recorded. One bad harness used to end a run of
        # hours at its first check.
        try:
            outs.append(ch.run(ctx))
        except Exception as error:  # noqa: BLE001 - any check failure
            print(f"check {ch.name} failed, no evidence recorded: {error!r}", file=sys.stderr)
    return outs


def cmd_history(cfg):
    rows = [r for r in T.read_jsonl(T.RESULTS)]
    cols = cfg.hooks.history_columns()
    print("| When | Levels | Score | " + "".join(f"{t} | " for t, _ in cols) + "Source | Commit |\n"
          + "|---" * (5 + len(cols)) + "|")
    for r in rows:
        sc = r.get("score") or {}
        print(f"| {r['at'][:16]} | {','.join(r.get('levels', []))} | {sc.get('score', '-')} | "
              + "".join(f"{R.fmt(sc.get(k))} | " for _, k in cols)
              + f"{r.get('source')} | {(r.get('commit') or '-')[:10]} |")
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
    cfg = C.load()
    names = level_names(cfg)
    ap = argparse.ArgumentParser(prog="verify", description=cfg.title,
                                 formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__)
    ap.add_argument("--level", choices=list(names))
    ap.add_argument("--points", help="comma-separated point ids")
    ap.add_argument("--quick", action="store_true", help="skip checks over 5 minutes")
    ap.add_argument("--plan", action="store_true")
    ap.add_argument("--score", action="store_true")
    ap.add_argument("--json", action="store_true", help="print report.json instead of report.md")
    ap.add_argument("--record", action="store_true")
    ap.add_argument("--history", action="store_true")
    ap.add_argument("--checkpoint")
    ap.add_argument("--against")
    ap.add_argument("--attempt")
    ap.add_argument("--kept", choices=["yes", "no"])
    ap.add_argument("--before", type=float)
    ap.add_argument("--after", type=float)
    ap.add_argument("--why")
    ap.add_argument("--compact", action="store_true")
    ap.add_argument("--out", help="evidence directory (default target/verifiers/<name>/<time>)")
    cfg.hooks.add_args(ap.add_argument_group("verifier options"))
    a = ap.parse_args(argv)

    runs = T.read_jsonl(T.RESULTS)
    checks = K.narrow(cfg, cfg.hooks.checks())
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

    level = names.get(a.level) if a.level else None
    env = {"runs": runs, "evidence": evidence, "started": started, "make_run": make_run, "level": level}
    current = cfg.hooks.command(cfg, a, env)
    if isinstance(current, int):
        return current
    if level and cfg.levels[level].get("paid") and current is None:
        ap.error(f"{a.level} is a paid level and this verifier has no runner for it")
    if current is None and not (a.plan or a.score or a.checkpoint or a.against):
        ctx = Ctx(cfg, runs, evidence, **cfg.hooks.context(cfg, a))
        os.makedirs(evidence, exist_ok=True)
        if level:
            levels = [level]
        else:
            levels = list(cfg.hooks.default_levels or [k for k, v in cfg.levels.items() if not v.get("paid")])
        chosen = [c for c in checks if c.level in levels and not (a.quick and c.minutes > 5)]
        # Higher levels first, so the lower ones (a model) see nothing of their own.
        chosen.sort(key=lambda c: c.level, reverse=True)
        outs = run_checks(ctx, chosen, points)
        current = make_run(cfg, outs, levels, "verify" + (f" --level {a.level}" if a.level else ""), started)

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
            print("\npaid:")
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
