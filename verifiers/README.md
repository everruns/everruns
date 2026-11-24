# Verifiers

A verifier is an executable requirement: one command that answers "does the
system meet this goal?" with a score from 0 to 100, the evidence behind it,
how sure that evidence is, and the cheapest next step that would raise it.

```
verifiers/
  lib/                 the engine: knows nothing about what is verified
    engine/            config, scoring, freshness, report and climb list,
                       journals, the CLI, Check/Output, hooks, runners
    tests/             the engine alone, with a toy verifier (pages-uk)
  <verifier>/
    goal.toml          what the score means (owner only, fingerprinted)
    levels.toml        levels of certainty
    points.toml        the points, their targets and weights
    layout.toml        the lever a climbing agent changes (optional)
    verifier.py        its hooks: checks, target names, extra rules, report
    checks/ ...        its own checks and tooling
    results.jsonl      its climbing record
    tests/, verify     its tests and entry point
```

Each verifier owns everything specific to it, even when two verifiers are
about the same system: they keep their own copies so their goals can
diverge. Only what any verifier could use goes in `lib/` (running a
command, reading a script's JSON, running cargo tests). A verifier without
`verifier.py` gets neutral hooks and only needs checks.

| Verifier | Goal |
|---|---|
| [agents-1m](agents-1m/README.md) | 1,000,000 agents in parallel on agentd (data plane v2): cells working together at extreme size |
| [agents-75k](agents-75k/README.md) | the 75k gate: efficiency per agent and general scalability, plus the reliability, durability and recoverability it must prove |

Design and reasoning: `knowledge/operations/verifiers.md`.
