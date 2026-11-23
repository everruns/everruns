# Validators

A validator is an executable non-functional requirement: one command that
answers "does the system meet this goal?" with a score from 0 to 100, the
evidence behind it, how sure that evidence is, and the cheapest next step
that would raise it. Each validator is a folder of what it means (goal,
layout, points, levels, its results journal and its own tests); the engine,
checks and coding workload they share live in `lib/`. A check covers the
points of whichever validator defines them, so one validator never runs
another's checks. The load tooling they drive stays where it is.

| Validator | Goal |
|---|---|
| [agents-1m](agents-1m/README.md) | 1,000,000 agents in parallel on agentd (data plane v2) |
| [agents-75k](agents-75k/README.md) | the 75k gate, plus the reliability, durability and recoverability it must prove |

| `lib/` | What |
|---|---|
| `engine/` | config, scoring, freshness, report and climb list, the store, the CLI |
| `checks/` | model, cluster simulation, realism, fleet tests, protocol models, local harnesses, fault proofs, cloud import and dispatch |
| `workload/` | `capture.py` (coding sessions to numbers), `coding-profile.json`, `generate.py` (llmsim scenarios) |

Design and reasoning: `knowledge/operations/validators.md`.
