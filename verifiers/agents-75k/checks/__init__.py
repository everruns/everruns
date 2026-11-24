"""Checks: each one runs (or reads) something and returns evidence entries.

A check never owns load tooling. It calls the harnesses that exist
(scripts/agentd/*, the agentd fleet tests, the SaaS load workflow) with the
goal's numbers and reads their output. When a harness changes its output, its
adapter changes in the same PR; tests/ feeds every adapter a recorded output.
"""

from dataclasses import dataclass

from engine import checks as K
from engine.checks import Check  # noqa: F401  (checks import it from here)


@dataclass
class Output(K.Output):
    agents: int = None                            # sessions the run held (L3/L4)
    density: float = None                         # sessions per shard the run reached
    coding_load: bool = False                     # the run played the coding scenarios


def all_checks():
    from . import cloud, cluster, fleettests, formal, local, model, realism, reliability
    out = []
    for mod in (model, cluster, realism, fleettests, formal, local, reliability, cloud):
        out.extend(mod.CHECKS)
    return out


def registry(cfg=None):
    """Every check, or with cfg the ones that cover its points at its levels."""
    return K.narrow(cfg, all_checks()) if cfg is not None else all_checks()


# Points no check covers yet, with what would cover them. The climb list
# offers "build the check" for each; when one is built it moves into a module.
NOT_BUILT = {
    "B2": ("L2", "agentd-drive peak profile: 3x rate for 10 minutes, shed share and retries"),
    "B3": ("L2", "agentd-drive open loop at the ramp rate (sessions created a second)"),
    "B4": ("L2", "drive reports with model time split out of turn time"),
    "D2": ("L2", "stream consumer reading the goal's agents' records from /v1/stream"),
    "D3": ("L2", "synthetic spend-grant requests at the goal's rate"),
    "E3": ("L2", "zone loss for 5 minutes with Chaos Mesh on the local cluster"),
    "E6": ("L2", "rolling upgrade during a run"),
    "F2": ("L1", "long-session gate: turn time flat over 200,000 turns"),
    "G1": ("L2", "memory_bench.py with the coding scenarios"),
    "G2": ("L2", "drive with the coding scenarios, overhead per model call"),
    "G3": ("L1", "idle unload and reload test (memory design)"),
    "G4": ("L1", "blob reference test for payloads over 256 KB (memory design)"),
    "G5": ("L2", "llmsim compaction scenario at shard scale"),
    "U12": ("L2", "disk full on a shard node: the node fences itself, the shard carries on, no corruption"),
    "U13": ("L2", "memory at 98%: controlled restart hands leadership over, no turn lost"),
}
