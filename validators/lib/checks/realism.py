"""L1 realism check (G6): the generated coding load matches the profile.

Reads the committed llmsim scenarios (crates/drivers/llmsim/scenarios/
coding-profile.json) and measures what they would play: model calls per
turn, tool output sizes and output tokens per turn, played with equal
weight. Each shape's median and p90 must be within the goal's tolerance
(10%) of the profile. A run that played a coding load which fails this is
invalid. It also fails when the scenarios are stale against the profile.
"""

import json
import os

from . import Check, Output
from workload import generate
from workload.capture import quantile

SHAPES = ["model_calls_per_turn", "tool_output_kb", "output_tokens_per_turn"]


def distance(profile, scens):
    got = generate.shapes_of(scens)
    worst, detail = 0.0, {}
    for k in SHAPES:
        want = profile["shapes"][k]
        for q, name in ((0.5, "median"), (0.9, "p90")):
            have = quantile(got[k], q)
            d = abs(have - want[name]) / want[name] if want[name] else 0.0
            detail[f"{k}.{name}"] = {"profile": want[name], "generated": round(have, 3), "distance": round(d, 3)}
            worst = max(worst, d)
    return worst, detail


def run(ctx):
    out = Output()
    path = generate.OUT
    if not os.path.exists(path):
        out.add("G6", "L1", 1.0, note="no generated scenarios: run workload/generate.py")
        return out
    with open(path) as f:
        scens = json.load(f)
    worst, detail = distance(ctx.profile, scens)
    with open(path) as f:
        stale = f.read() != generate.render(ctx.profile)
    if stale:
        worst = max(worst, 1.0)
    out.add("G6", "L1", round(worst, 3),
            note=("scenarios are stale against the profile; " if stale else "")
            + f"{len(scens)} scenarios, worst shape distance {worst:.1%}")
    out.metrics["realism"] = detail
    src = ctx.profile.get("source", {})
    out.metrics["coding_profile"] = {"sessions": src.get("sessions"), "turns": src.get("turns"),
                                     "captured": src.get("captured")}
    return out


CHECKS = [Check("realism", "L1", ["G6"], minutes=0.1,
                command="validate (runs with every default run)", run=run)]
