"""Scoring: closeness (m) x certainty (c) x freshness (f), kept at each
point's best level, weighted to a 0-100 score.

Rules (knowledge/operations/validators.md):
- A failed gate caps the score at 25. The newest gate result wins.
- Noise does not count: with samples, m is taken at the unfavourable end.
- Invalid runs earn nothing: another goal fingerprint, a saturated load
  generator (D7), a coding load off its profile (G6), per-shard evidence
  below goal density, shared-part evidence from a run smaller than the goal.
- The model level never gives more than its certainty (0.15) per point.
"""

import datetime as dt
import statistics
from dataclasses import dataclass

from . import config as C

GATE_CAP = 25.0
STALE_DAYS = 30


def parse_time(s):
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


def now_utc():
    return dt.datetime.now(dt.timezone.utc)


def median_value(entry):
    s = entry.get("samples")
    if s:
        return statistics.median(s)
    return entry.get("value")


def closeness(point, entry, target):
    """m in [0, 1], at the unfavourable end of the samples."""
    if point.direction == "pass":
        return 1.0 if entry.get("value") is True else 0.0
    samples = entry.get("samples") or [entry.get("value")]
    if any(v is None for v in samples):
        return 0.0
    if point.direction == "at_least":
        v = min(samples)
        return 1.0 if target <= 0 else max(0.0, min(1.0, v / target))
    v = max(samples)
    if target <= 0:
        return 1.0 if v <= target else 0.0
    return 1.0 if v <= 0 else max(0.0, min(1.0, target / v))


def passes(point, entry, target):
    return closeness(point, entry, target) >= 1.0


def exclusion(cfg, run, point, entry, agents):
    """Why this entry earns nothing, or None."""
    level = entry["level"]
    if run.get("fingerprint") != cfg.fingerprint:
        return "recorded under another goal fingerprint"
    if run.get("invalid"):
        return "run invalid: " + "; ".join(run["invalid"])
    lvl = C.LEVELS.index(level)
    density = entry.get("density", run.get("density"))
    want = cfg.goal["density"]["sessions_per_shard"]
    if point.scales == "per_shard" and lvl >= 2 and (density is None or density < 0.95 * want):
        return f"below goal density ({density or 0:,.0f} < {want:,} sessions per shard)"
    size = entry.get("agents", run.get("agents")) or 0
    if point.scales == "shared" and lvl >= 3 and size < agents:
        return f"shared part from a run of {size:,} agents, below {agents:,}"
    return None


def run_validity(cfg, run):
    """Validity points decide whether a whole run counts (filled in at record)."""
    reasons = []
    for e in run.get("entries", []):
        p = cfg.point(e["point"]) if e["point"] in {q.id for q in cfg.points} else None
        if p is None or not p.validity:
            continue
        if p.id == "G6" and not run.get("coding_load", True):
            continue
        if not passes(p, e, cfg.target(p)):
            reasons.append(f"{p.id} failed ({p.title.lower()})")
    return reasons


@dataclass
class Cell:
    """A point's newest valid evidence at one level."""
    level: str
    entry: dict
    run: dict
    m: float
    c: float
    f: float
    target: float

    @property
    def credit(self):
        return self.m * self.c * self.f


@dataclass
class PointState:
    point: C.Point
    target: float
    cells: dict          # level -> Cell
    excluded: list       # (level, run id, reason)
    gate_failed: bool = False
    gate_entry: dict = None

    @property
    def best(self):
        return max(self.cells.values(), key=lambda c: c.credit, default=None)

    @property
    def credit(self):
        b = self.best
        return b.credit if b else 0.0


class Freshness:
    """f = 1, halved when the code a point watches changed since its evidence
    (the entry's digest of the watched files, else its commit), halved again
    when the evidence is older than 30 days."""

    def __init__(self, changed=None, now=None, current=None):
        self._changed = changed or (lambda commit, paths: True)
        self._current = current          # paths -> digest of the working tree
        self._cache = {}
        self.now = now or now_utc()

    def changed(self, commit, paths, digest=None):
        if digest and self._current:
            key = ("digest", tuple(paths))
            if key not in self._cache:
                self._cache[key] = self._current(paths)
            return self._cache[key] != digest
        key = (commit, tuple(paths))
        if key not in self._cache:
            self._cache[key] = True if not commit else self._changed(commit, paths)
        return self._cache[key]

    def __call__(self, point, run, entry=None):
        f = 1.0
        if self.changed(run.get("commit"), point.watches, (entry or {}).get("watch")):
            f *= 0.5
        if (self.now - parse_time(run["at"])).days > STALE_DAYS:
            f *= 0.5
        return f


def states(cfg, runs, fresh, agents=None):
    agents = agents or cfg.derived()["agents"]
    out = {}
    for p in cfg.points:
        target = cfg.target(p, agents)
        st = PointState(p, target, {}, [])
        newest_gate = None
        for run in sorted(runs, key=lambda r: r["at"]):
            for e in run.get("entries", []):
                if e["point"] != p.id:
                    continue
                why = exclusion(cfg, run, p, e, agents)
                if why:
                    st.excluded.append((e["level"], run["run"], why))
                    continue
                m = closeness(p, e, target)
                c = cfg.certainty(e["level"])
                st.cells[e["level"]] = Cell(e["level"], e, run, m, c, fresh(p, run, e), target)
                if p.gate:
                    newest_gate = (e, m)
        if p.gate and newest_gate is not None:
            st.gate_entry = newest_gate[0]
            st.gate_failed = newest_gate[1] < 1.0
        out[p.id] = st
    return out


def total(cfg, sts):
    w = cfg.total_weight
    raw = 100.0 * sum(st.point.weight * st.credit for st in sts.values()) / w
    failed = [pid for pid, st in sts.items() if st.gate_failed]
    return (min(raw, GATE_CAP) if failed else raw), raw, failed


def proven_agents(cfg, runs):
    """Largest L3/L4 run whose gate results all passed (and had at least one)."""
    best = 0
    gates = {p.id for p in cfg.points if p.gate}
    for run in runs:
        if run.get("fingerprint") != cfg.fingerprint or run.get("invalid"):
            continue
        es = [e for e in run.get("entries", []) if e["point"] in gates and e["level"] in ("L3", "L4")]
        if es and all(passes(cfg.point(e["point"]), e, cfg.target(cfg.point(e["point"]))) for e in es):
            best = max(best, run.get("agents") or 0)
    return best


def milestones(cfg, runs, fresh):
    """Per level, the highest milestone where the gates are evidenced at that
    level or above, none failed, and nothing measured at that level or above
    misses the milestone's targets."""
    gates = [p.id for p in cfg.points if p.gate]
    out = {}
    for li, level in enumerate(C.LEVELS):
        reached = None
        for n in cfg.goal["milestones"]["agents"]:
            sts = states(cfg, runs, fresh, agents=n)
            if any(st.gate_failed for st in sts.values()):
                break
            ok = all(any(C.LEVELS.index(l) >= li for l in sts[g].cells) for g in gates)
            for st in sts.values():
                if st.point.validity:
                    continue
                for lvl, cell in st.cells.items():
                    if C.LEVELS.index(lvl) >= li and cell.m < 1.0:
                        ok = False
            if not ok:
                break
            reached = n
        out[level] = reached
    return out
