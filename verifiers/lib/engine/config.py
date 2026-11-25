"""Loading goal.toml, layout.toml, levels.toml and points.toml.

Decisions:
- Targets are expressions over goal.*, layout.* and g.* so a goal change, a
  layout change or a milestone moves every target with it. g.* comes from
  the verifier's hooks (engine/hooks.py); the engine itself defines none.
  They are evaluated by a tiny arithmetic evaluator, never eval().
- layout.toml is optional: only verifiers with a lever to tune need one.
- The fingerprint covers the parsed goal minus its [approval] and [meta]
  tables, so the owner can record an approval or retitle the verifier
  without invalidating results.
- A goal edit invalidates every result by default. The owner can list
  earlier fingerprints in [approval] carry_from: their results are then
  scored again against today's targets (raw values are recorded, so a
  moved limit re-judges them). points.toml edits already work that way.
  Only the owner writes goal.toml, so only the owner can carry evidence.
"""

import ast
import hashlib
import importlib.util
import json
import math
import operator
import os
import sys
import tomllib
from dataclasses import dataclass, field

LIB = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO = os.path.dirname(os.path.dirname(LIB))
# The verifier folder (goal.toml, points.toml, results.jsonl): its `verify`
# script sets VERIFIER_ROOT before importing the engine.
ROOT = os.path.abspath(os.environ.get("VERIFIER_ROOT") or os.getcwd())
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
    target: str = None
    watches: list = field(default_factory=list)   # paths whose change halves freshness
    cheapest: str = ""     # the cheapest level that can evidence it
    built_from: str = ""
    scales: str = ""       # verifier-defined; the engine does not read it
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
    hooks: object = None
    size: float = None  # a milestone overrides the verifier's default size
    _by_id: dict = field(default_factory=dict, repr=False)

    def accepts(self, fp):
        """Whether a result recorded under goal fingerprint fp counts."""
        return fp == self.fingerprint or fp in carried(self.goal)

    def point(self, pid):
        if not self._by_id:
            self._by_id = {p.id: p for p in self.points}
        return self._by_id[pid]

    @property
    def total_weight(self):
        return sum(p.weight for p in self.points)

    @property
    def title(self):
        return self.goal.get("meta", {}).get("title") or f"{NAME} verifier"

    def has(self, pid):
        return any(p.id == pid for p in self.points)

    def certainty(self, level):
        return self.levels[level]["certainty"]

    def default_size(self):
        return self.size or self.hooks.default_size(self)

    def derived(self, size=None):
        return self.hooks.derived(self, size or self.default_size())

    def names(self, size=None):
        return {"goal": NS(self.goal), "layout": NS(self.layout), "g": NS(self.derived(size))}

    def target(self, point, size=None):
        if point.target is None:
            return None
        return float(evaluate(point.target, self.names(size)))

    def at(self, size):
        """The same config scored at a milestone."""
        return Config(self.goal, self.layout, self.levels, self.points, self.fingerprint, self.hooks, size)


def carried(goal):
    """Earlier goal fingerprints whose results the owner carried over."""
    return set((goal.get("approval") or {}).get("carry_from") or [])


def fingerprint(goal):
    body = {k: v for k, v in goal.items() if k not in ("approval", "meta")}
    return hashlib.sha256(json.dumps(body, sort_keys=True).encode()).hexdigest()[:16]


def load_hooks(root):
    """The verifier's own verifier.py (its VERIFIER), else the neutral hooks.
    The verifier folder joins sys.path, so its checks import as `checks`."""
    from .hooks import BASE
    own = os.path.join(root, "verifier.py")
    if not os.path.exists(own):
        return BASE
    if root not in sys.path:
        sys.path.insert(0, root)
    spec = importlib.util.spec_from_file_location(f"verifier_{os.path.basename(root).replace('-', '_')}", own)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.VERIFIER


def load(root=ROOT):
    goal = load_toml(os.path.join(root, "goal.toml"))
    lp = os.path.join(root, "layout.toml")
    layout = load_toml(lp) if os.path.exists(lp) else {}
    levels = load_toml(os.path.join(root, "levels.toml"))
    points = [Point(**p) for p in load_toml(os.path.join(root, "points.toml"))["point"]]
    cfg = Config(goal, layout, levels, points, fingerprint(goal), load_hooks(root))
    for p in points:  # every target must evaluate
        cfg.target(p)
        if p.direction not in ("at_least", "at_most", "pass"):
            raise ValueError(f"{p.id}: unknown direction {p.direction}")
        if p.direction != "pass" and p.target is None:
            raise ValueError(f"{p.id}: needs a target")
    return cfg
