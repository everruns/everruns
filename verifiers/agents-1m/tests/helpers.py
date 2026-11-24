import copy

from engine import config as C
from engine import score as S

CFG = C.load()
NOW = S.parse_time("2026-11-23T20:00:00Z")


def fresh(changed=False):
    return S.Freshness(changed=lambda commit, paths: changed, now=NOW)


def run(entries, at="2026-11-23T12:00:00Z", density=25_000, agents=None, levels=None, **kw):
    r = {"run": f"r-{at}", "at": at, "commit": "abc", "fingerprint": CFG.fingerprint,
         "levels": levels or sorted({e["level"] for e in entries}), "density": density,
         "agents": agents, "entries": copy.deepcopy(entries), "metrics": kw.pop("metrics", {}), "invalid": []}
    r.update(kw)
    r["invalid"] = r["invalid"] or S.run_validity(CFG, r)
    return r


def e(point, level, value, **kw):
    return {"point": point, "level": level, "value": value, **kw}
