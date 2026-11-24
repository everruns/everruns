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
    from . import cloud, cluster, fleettests, local, model
    out = []
    for mod in (model, cluster, fleettests, local, cloud):
        out.extend(mod.CHECKS)
    return out


def registry(cfg=None):
    """Every check, or with cfg the ones that cover its points at its levels."""
    return K.narrow(cfg, all_checks()) if cfg is not None else all_checks()


# Points no check covers yet, with what would cover them. The climb list
# offers "build the check" for each; when one is built it moves into a module.
NOT_BUILT = {
    "B2": ("L2", "agentd-drive peak profile across shards: 3x rate for 10 minutes, shed share and retries"),
    "D2": ("L2", "stream consumer reading the goal's agents' records from /v1/stream"),
    "D3": ("L2", "synthetic spend-grant requests at the goal's rate"),
    "D4": ("L1", "in-process directory and edge router holding 1M sessions"),
    "X2": ("L1", "edge router forwarding turns at the goal's rate in process, added latency p99"),
    "X3": ("L2", "drive load across shards while sessions move between them: lost or repeated messages, move p99"),
    "X4": ("L2", "one hot shard among many with the rebalancer on, under drive load: turn errors"),
    "D6": ("L1", "KWOK cluster with hundreds of shards and ~700 pods"),
    "X6": ("L1", "directory and edge router while shards join, leave and fail, in process"),
    "X7": ("L2", "parents and their subagents placed on different shards under load: lost results, result p99"),
    "E3": ("L2", "zone loss for 5 minutes with Chaos Mesh on the local cluster"),
    "E6": ("L2", "rolling upgrade during a run"),
}
