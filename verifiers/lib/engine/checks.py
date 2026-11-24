"""What a check is and what it returns.

A check runs (or reads) something and returns evidence entries for points.
Each verifier defines its checks; the engine only runs them and scores what
they return.
"""

import dataclasses
import shutil
from dataclasses import dataclass, field


@dataclass
class Output:
    entries: list = field(default_factory=list)   # {point, level, value, samples?, note?, evidence?}
    metrics: dict = field(default_factory=dict)   # numbers the verifier reports or climbs on
    notes: list = field(default_factory=list)
    cost_usd: float = 0.0

    def add(self, point, level, value, **kw):
        self.entries.append({"point": point, "level": level, "value": value, **kw})


@dataclass
class Check:
    name: str
    level: str
    points: list
    minutes: float
    command: str
    run: object = None          # (ctx) -> Output
    needs: tuple = ()           # executables that must be on PATH
    cost_usd: float = 0.0
    built: bool = True

    def available(self):
        if not self.built:
            return False, "not built yet"
        missing = [n for n in self.needs if shutil.which(n) is None]
        if missing:
            return False, "needs " + ", ".join(missing)
        return True, ""


def narrow(cfg, checks):
    """The checks that cover at least one of cfg's points at a level it
    defines, their point lists narrowed to its points."""
    out = []
    for ch in checks:
        pts = [p for p in ch.points if cfg.has(p)]
        if pts and ch.level in cfg.levels:
            est = cfg.levels[ch.level].get("usd_estimate")
            out.append(dataclasses.replace(ch, points=pts, cost_usd=est[1] if est and ch.cost_usd else ch.cost_usd))
    return out
