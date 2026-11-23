"""Checks: each one runs (or reads) something and returns evidence entries.

A check never owns load tooling. It calls the harnesses that exist
(scripts/agentd/*, the agentd fleet tests, the SaaS load workflow) with the
goal's numbers and reads their output. When a harness changes its output, its
adapter changes in the same PR; tests/ feeds every adapter a recorded output.
"""

import shutil
from dataclasses import dataclass, field


@dataclass
class Output:
    entries: list = field(default_factory=list)   # {point, level, value, samples?, note?, evidence?}
    metrics: dict = field(default_factory=dict)   # per-agent costs and other climbing numbers
    notes: list = field(default_factory=list)
    agents: int = None                            # sessions the run held (L3/L4)
    density: float = None                         # sessions per shard the run reached
    coding_load: bool = False                     # the run played the coding scenarios
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


def registry():
    from . import cloud, cluster, fleettests, local, model, realism
    checks = []
    for mod in (model, cluster, realism, fleettests, local, cloud):
        checks.extend(mod.CHECKS)
    return checks


# Points no check covers yet, with what would cover them. The climb list
# offers "build the check" for each; when one is built it moves into a module.
NOT_BUILT = {
    "B2": ("L2", "agentd-drive peak profile: 3x rate for 10 minutes, shed share and retries"),
    "B3": ("L2", "agentd-drive open loop at the ramp rate (sessions created a second)"),
    "B4": ("L2", "drive reports with model time split out of turn time"),
    "D2": ("L2", "synthetic feed generator: 1M agents' records into the control plane"),
    "D3": ("L2", "synthetic spend-grant requests at the 1M rate"),
    "D4": ("L1", "in-process directory and edge router holding 1M sessions"),
    "D6": ("L1", "KWOK cluster with hundreds of shards and ~700 pods"),
    "E3": ("L2", "zone loss for 5 minutes with Chaos Mesh on the local cluster"),
    "E6": ("L2", "rolling upgrade during a run"),
    "F2": ("L1", "long-session gate: turn time flat over 200,000 turns"),
    "G1": ("L2", "memory_bench.py with the coding scenarios"),
    "G2": ("L2", "drive with the coding scenarios, overhead per model call"),
    "G3": ("L1", "idle unload and reload test (memory design)"),
    "G4": ("L1", "blob reference test for payloads over 256 KB (memory design)"),
    "G5": ("L2", "llmsim compaction scenario at shard scale"),
}
