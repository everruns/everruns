"""The report: report.json for agents, report.md for people.

Order: headline, verifier sections, milestones, verifier sections, climb list,
points, excluded evidence, inputs and notes, evidence. An agent reading it should know
what is failing, why, and which single action raises the score most for the
least time and money.
"""

import json

from . import score as S

DEFAULT_FIX_HOURS = 8.0      # agent time to close a measured gap, for ranking
DEFAULT_BUILD_HOURS = 4.0    # agent time to build a missing check


def fmt(v):
    if v is None:
        return "-"
    if isinstance(v, bool):
        return "pass" if v else "fail"
    if isinstance(v, float):
        if v != 0 and abs(v) < 0.01:
            return f"{v:.2e}"
        return f"{v:,.3f}".rstrip("0").rstrip(".") if abs(v) < 100 else f"{v:,.0f}"
    return f"{v:,}" if isinstance(v, int) else str(v)


def latest_metric(runs, key):
    for r in sorted(runs, key=lambda r: r["at"], reverse=True):
        v = (r.get("metrics") or {}).get(key)
        if v is not None:
            return v, r["run"]
    return None, None


def climb(cfg, sts, checks, uncapped, failed_gates):
    """Every action that raises the score, with expected gain, time and cost."""
    W = cfg.total_weight
    acts = []
    by_point = {}
    for ch in checks:
        for p in ch.points:
            by_point.setdefault(p, []).append(ch)
    NOT_BUILT = cfg.hooks.not_built()
    # Run a check at a higher level: one action per check, summed over its points.
    for ch in checks:
        if ch.minutes < 1:
            continue  # runs with every default run already
        gain, pts = 0.0, []
        for pid in ch.points:
            st = sts.get(pid)
            if not st or not st.point.weight:
                continue
            m_known = st.best.m if st.best else 1.0
            g = 100 * st.point.weight * (m_known * cfg.certainty(ch.level) - st.credit) / W
            if g > 0.01:
                gain += g
                pts.append(pid)
        if pts:
            ok, why = ch.available()
            acts.append({"kind": "run", "point": pts[0], "points": pts, "level": ch.level, "check": ch.name,
                         "gain": gain, "hours": ch.minutes / 60, "usd": ch.cost_usd,
                         "command": ch.command, "available": ok, "why": why,
                         "what": f"run {ch.name} ({ch.level} {cfg.levels[ch.level]['name']}) for {', '.join(pts)}"})
    for pid, st in sts.items():
        p = st.point
        best = st.best
        m_known = best.m if best else 1.0
        if pid in NOT_BUILT and not by_point.get(pid):
            level, what = NOT_BUILT[pid]
            gain = 100 * p.weight * cfg.certainty(level) / W
            if p.weight:
                acts.append({"kind": "build", "point": pid, "level": level, "gain": gain,
                             "hours": DEFAULT_BUILD_HOURS, "usd": 0, "command": None, "available": True,
                             "what": f"build the check for {pid}: {what}"})
        if best and best.m < 1.0 and p.weight:
            acts.append({"kind": "fix", "point": pid, "level": best.level,
                         "gain": 100 * p.weight * (1 - best.m) * best.c * best.f / W,
                         "hours": DEFAULT_FIX_HOURS, "usd": 0, "command": None, "available": True,
                         "what": f"close the gap on {pid}: {fmt(S.median_value(best.entry))} against {fmt(st.target)} {p.unit}".strip()})
        if best and best.f < 1.0 and p.weight:
            ch = next((c for c in by_point.get(pid, []) if c.level == best.level), None)
            acts.append({"kind": "refresh", "point": pid, "level": best.level,
                         "gain": 100 * p.weight * best.m * best.c * (1 - best.f) / W,
                         "hours": (ch.minutes / 60) if ch else 1.0, "usd": ch.cost_usd if ch else 0,
                         "command": ch.command if ch else None, "available": bool(ch and ch.available()[0]),
                         "what": f"refresh {pid}: code it depends on changed since its evidence"})
    for a in acts:
        if set(a.get("points", [a["point"]])) & set(failed_gates) and a["kind"] in ("fix", "run", "refresh"):
            a["gain"] += max(0.0, uncapped - S.GATE_CAP)
            a["lifts_gate_cap"] = True
        a["gain"] = round(a["gain"], 2)
        a["per_hour"] = round(a["gain"] / max(a["hours"], 0.05), 2)
    # A gate failure outranks everything: nothing else counts until it lifts.
    acts.sort(key=lambda a: (not a.get("lifts_gate_cap"), -a["per_hour"]))
    free = [a for a in acts if not a["usd"]]
    paid = sorted([a for a in acts if a["usd"]], key=lambda a: -a["gain"] / a["usd"])
    return free, paid


def build(cfg, runs, fresh, checks, current=None):
    sts = S.states(cfg, runs, fresh)
    value, uncapped, failed = S.total(cfg, sts)
    free, paid = climb(cfg, sts, checks, uncapped, failed)
    recorded = [r for r in runs if r.get("recorded")]
    last = max(recorded, key=lambda r: r["at"]) if recorded else None
    points = []
    for pid, st in sts.items():
        b = st.best
        points.append({
            "id": pid, "title": st.point.title, "weight": st.point.weight, "gate": st.point.gate,
            "validity": st.point.validity, "target": st.target, "unit": st.point.unit,
            "best_level": b.level if b else None,
            "value": S.median_value(b.entry) if b else None,
            "m": round(b.m, 3) if b else None, "c": b.c if b else None, "f": b.f if b else None,
            "credit": round(st.point.weight * st.credit, 3),
            "levels": {l: {"value": S.median_value(c.entry), "m": round(c.m, 3), "f": c.f,
                           "run": c.run["run"], "note": c.entry.get("note"), "evidence": c.entry.get("evidence")}
                       for l, c in sorted(st.cells.items())},
            "gate_failed": st.gate_failed,
            "note": b.entry.get("note") if b else None,
            "excluded": [{"level": l, "run": r, "why": w} for l, r, w in st.excluded][-3:],
        })
    rep = {
        "score": round(value, 2), "uncapped": round(uncapped, 2), "failed_gates": failed,
        "fingerprint": cfg.fingerprint,
        "commit": (current or {}).get("commit"),
        "since_last_recorded": round(value - last["score"]["score"], 2) if last and last.get("score") else None,
        "milestones": S.milestones(cfg, runs, fresh),
        "climb": free[:25], "paid": paid[:10],
        "points": points,
        "notes": (current or {}).get("notes", []),
    }
    rep.update(cfg.hooks.summary(cfg, runs, fresh))
    return rep


def markdown(rep, cfg):
    L = []
    w = L.append
    cap = f" (capped from {rep['uncapped']} by failed gates {', '.join(rep['failed_gates'])})" if rep["failed_gates"] else ""
    w(f"# {cfg.title}\n")
    extra = cfg.hooks.headline(cfg, rep)
    w(f"**Score {rep['score']} / 100**{cap}." + (f" {extra}" if extra else ""))
    w(f"Goal fingerprint `{rep['fingerprint']}`, commit `{(rep['commit'] or '-')[:10]}`"
      + (f", {rep['since_last_recorded']:+} since the last recorded run." if rep["since_last_recorded"] is not None else "."))
    L.extend(cfg.hooks.sections(cfg, rep))
    if rep["milestones"]:
        w("\n## Milestones\n")
        w(f"| Level | Highest milestone that passes ({cfg.hooks.size_unit}) |\n|---|---|")
        for lvl, n in rep["milestones"].items():
            w(f"| {lvl} {cfg.levels[lvl]['name']} | {fmt(n) if n else 'none yet'} |")
    L.extend(cfg.hooks.after_milestones(cfg, rep))
    w("\n## Climb list (free)\n")
    w("| # | Action | Gain | Hours | Command |\n|---|---|---|---|---|")
    for i, a in enumerate(rep["climb"], 1):
        flag = " (lifts the gate cap)" if a.get("lifts_gate_cap") else ""
        avail = "" if a.get("available", True) else f" [{a.get('why')}]"
        w(f"| {i} | {a['what']}{flag}{avail} | +{a['gain']} | {a['hours']:.1f} | {('`' + a['command'] + '`') if a['command'] else '-'} |")
    if rep["paid"]:
        w("\n## Paid actions\n")
        w("| Action | Gain | Cost | Command |\n|---|---|---|---|")
        for a in rep["paid"]:
            w(f"| {a['what']} | +{a['gain']} | ~${a['usd']} | `{a['command']}` |")
    w("\n## Points\n")
    w("| ID | Point | W | Best | Value | Target | m | c | f | Credit |\n|---|---|---|---|---|---|---|---|---|---|")
    for p in rep["points"]:
        tag = " (gate)" if p["gate"] else (" (validity)" if p["validity"] else "")
        bad = " **FAILED**" if p["gate_failed"] else ""
        w(f"| {p['id']} | {p['title']}{tag}{bad} | {fmt(p['weight'])} | {p['best_level'] or '-'} | "
          f"{fmt(p['value'])} | {fmt(p['target']) if p['target'] is not None else 'pass'} {p['unit']} | "
          f"{fmt(p['m'])} | {fmt(p['c'])} | {fmt(p['f'])} | {p['credit']:.2f} |")
    ex = [(p["id"], e) for p in rep["points"] for e in p["excluded"]]
    if ex:
        w("\n## Evidence that earned nothing\n")
        for pid, e in ex:
            w(f"- {pid} {e['level']} ({e['run']}): {e['why']}")
    w("\n## Inputs, assumptions and notes\n")
    L.extend(cfg.hooks.notes(cfg, rep))
    for n in rep["notes"]:
        w(f"- {n}")
    w("\n## Evidence\n")
    for p in rep["points"]:
        for lvl, c in p["levels"].items():
            line = f"- {p['id']} {lvl}: {fmt(c['value'])} from {c['run']}"
            if c.get("note"):
                line += f"; {c['note']}"
            if c.get("evidence"):
                line += f" (`{c['evidence']}`)"
            w(line)
    return "\n".join(L) + "\n"


def to_json(rep):
    return json.dumps(rep, indent=1, default=str) + "\n"
