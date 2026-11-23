"""Loading goal.toml, layout.toml, levels.toml and points.toml.

Decisions:
- Targets are expressions over goal.*, layout.* and g.* so a goal change, a
  layout change or a milestone (fewer agents) moves every target with it.
  They are evaluated by a tiny arithmetic evaluator, never eval().
- The fingerprint covers the parsed goal minus its [approval] and [meta]
  tables, so the owner can record an approval or retitle the validator
  without invalidating results.
"""

import ast
import hashlib
import json
import math
import operator
import os
import tomllib
from dataclasses import dataclass, field

LIB = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO = os.path.dirname(os.path.dirname(LIB))
# The validator folder (goal.toml, points.toml, results.jsonl): its `validate`
# script sets VALIDATOR_ROOT before importing the engine.
ROOT = os.path.abspath(os.environ.get("VALIDATOR_ROOT") or os.path.join(REPO, "validators", "agents-1m"))
NAME = os.path.basename(ROOT)
LEVELS = ["L0", "L1", "L2", "L3", "L4"]


def load_toml(path):
    with open(path, "rb") as f:
        return tomllib.load(f)


class NS:
    """Attribute access over a nested dict, for target expressions."""

    def __init__(self, d):
        self._d = d

    def __getattr__(self, name):
        try:
            v = self._d[name]
        except KeyError:
            raise AttributeError(name) from None
        return NS(v) if isinstance(v, dict) else v


_OPS = {
    ast.Add: operator.add, ast.Sub: operator.sub, ast.Mult: operator.mul,
    ast.Div: operator.truediv, ast.Pow: operator.pow, ast.USub: operator.neg,
    ast.UAdd: operator.pos,
}
_FUNCS = {"min": min, "max": max, "ceil": math.ceil}


def evaluate(expr, names):
    """Arithmetic over numbers and dotted names; nothing else is allowed."""

    def ev(node):
        if isinstance(node, ast.Expression):
            return ev(node.body)
        if isinstance(node, ast.Constant) and isinstance(node.value, (int, float)):
            return node.value
        if isinstance(node, ast.BinOp) and type(node.op) in _OPS:
            return _OPS[type(node.op)](ev(node.left), ev(node.right))
        if isinstance(node, ast.UnaryOp) and type(node.op) in _OPS:
            return _OPS[type(node.op)](ev(node.operand))
        if isinstance(node, ast.Name) and node.id in names:
            return names[node.id]
        if isinstance(node, ast.Attribute) and not node.attr.startswith("_"):
            base = ev(node.value)
            if not isinstance(base, NS):
                raise ValueError(f"not allowed in a target: .{node.attr}")
            return getattr(base, node.attr)
        if (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                and node.func.id in _FUNCS and not node.keywords):
            return _FUNCS[node.func.id](*[ev(a) for a in node.args])
        raise ValueError(f"not allowed in a target: {ast.dump(node)}")

    return ev(ast.parse(str(expr), mode="eval"))


@dataclass
class Point:
    id: str
    title: str
    weight: float
    direction: str
    cheapest: str
    scales: str
    built_from: str
    watches: list
    target: str = None
    unit: str = ""
    gate: bool = False
    validity: bool = False

    @property
    def group(self):
        return self.id[0]


@dataclass
class Config:
    goal: dict
    layout: dict
    levels: dict
    points: list
    fingerprint: str
    agents: int = None  # a milestone overrides the goal's agent count
    _by_id: dict = field(default_factory=dict, repr=False)

    def point(self, pid):
        if not self._by_id:
            self._by_id = {p.id: p for p in self.points}
        return self._by_id[pid]

    @property
    def total_weight(self):
        return sum(p.weight for p in self.points)

    @property
    def title(self):
        return self.goal.get("meta", {}).get("title") or f"{NAME} validator"

    def full_size(self, agents=None):
        """Agents a run needs for shared points: the goal's (or milestone's)
        count times [agents] full_run_min_share (1.0 unless the goal says)."""
        n = agents or self.agents or self.goal["agents"]["total"]
        return n * self.goal["agents"].get("full_run_min_share", 1.0)

    def has(self, pid):
        return any(p.id == pid for p in self.points)

    def certainty(self, level):
        return self.levels[level]["certainty"]

    def derived(self, agents=None):
        g = self.goal
        n = agents or self.agents or g["agents"]["total"]
        pace = g["pace"]
        return {
            "agents": n,
            "coding_agents": n * g["agents"]["coding_share"],
            "turns_per_sec": n / pace["turn_every_s"],
            "peak_turns_per_sec": n / pace["turn_every_s"] * pace["peak_multiplier"],
            "ramp_per_sec": n / (pace["ramp_minutes"] * 60),
            "shards": math.ceil(n / g["density"]["sessions_per_shard"]),
        }

    def names(self, agents=None):
        return {"goal": NS(self.goal), "layout": NS(self.layout), "g": NS(self.derived(agents))}

    def target(self, point, agents=None):
        if point.target is None:
            return None
        return float(evaluate(point.target, self.names(agents)))

    def at(self, agents):
        """The same config scored at a milestone."""
        return Config(self.goal, self.layout, self.levels, self.points, self.fingerprint, agents)


def fingerprint(goal):
    body = {k: v for k, v in goal.items() if k not in ("approval", "meta")}
    return hashlib.sha256(json.dumps(body, sort_keys=True).encode()).hexdigest()[:16]


def load(root=ROOT):
    goal = load_toml(os.path.join(root, "goal.toml"))
    layout = load_toml(os.path.join(root, "layout.toml"))
    levels = load_toml(os.path.join(root, "levels.toml"))
    points = [Point(**p) for p in load_toml(os.path.join(root, "points.toml"))["point"]]
    cfg = Config(goal, layout, levels, points, fingerprint(goal))
    for p in points:  # every target must evaluate
        cfg.target(p)
        if p.direction not in ("at_least", "at_most", "pass"):
            raise ValueError(f"{p.id}: unknown direction {p.direction}")
        if p.direction != "pass" and p.target is None:
            raise ValueError(f"{p.id}: needs a target")
    return cfg
