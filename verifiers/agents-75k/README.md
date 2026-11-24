# agents-75k: does Everruns hold 75,000 agents, and survive what goes wrong?

The 75k gate as a verifier: efficiency per agent and general scalability at
75k (3 shards at 25,000 sessions each), plus what a gate run alone cannot
show: that agentd stays correct when nodes, executors, shards, zones and
the store fail, that it restores what it promises to restore, and that the
gate passes run after run. Target: agentd (data plane v2) only; the Everruns server is out of
scope.

```bash
verifiers/agents-75k/verify --plan     # what to do next (seconds, runs nothing)
verifiers/agents-75k/verify --quick    # model + simulation levels (seconds)
verifiers/agents-75k/verify            # plus fleet tests and protocol models (~30 min)
verifiers/agents-75k/verify --level local --points R4   # one fault proof
```

Python 3.11+ standard library only. The report lands in
`target/verifiers/agents-75k/<time>/report.md` (and `report.json`) and is
printed. `just verify agents-75k <args>` is the same.

## Reading this verifier

In this order; nothing outside this folder is needed except the engine.

| File | What |
|---|---|
| `README.md` | this page: the goal, how to climb, the rules |
| `goal.toml` | what 75k means: agents, pace, density, failures, limits (owner only, fingerprinted) |
| `points.toml` | the 58 points: target, weight, gate or validity, the files each watches |
| `levels.toml` | the four levels and their certainty |
| `layout.toml` | the fleet the goal is projected onto (a lever) |
| `verifier.py` | what this verifier adds to the engine: agents and shards as target names, the density and run-size rules, proven agents, gate run import and the paid dispatch, its report sections |
| `checks/` | one module per kind of evidence: `model`, `cluster`, `realism`, `fleettests`, `formal`, `local`, `reliability`, `cloud`; `__init__.py` lists them and the points not built yet |
| `workload/` | the coding profile (numbers only) and the llmsim scenarios generated from it |
| `results.jsonl`, `attempts.jsonl`, `checkpoints/` | the committed climbing record |
| `tests/`, `verify` | `python3 -m unittest discover -s tests -t .`, and the entry point |

The engine in `../lib/` (scoring, freshness, report, climb list, CLI) knows
nothing about agents; this folder is the whole of what 75k means. agents-1m
has its own copy of the agentd checks, because its goal is a different one.

## Climbing the score

1. `verify --plan`: the climb list, ranked by points gained per hour.
2. Take the top free item: "run X" runs a check at a higher level, "close
   the gap" fixes a measured shortfall in the code, "build the check" adds a
   check for a point nothing measures yet (`checks/__init__.py` NOT_BUILT).
3. Re-run only what you touched: `verify --level local --points R4`.
4. Keep the change only if the score rose and no gate fell:
   `verify --checkpoint before`, then `verify --against before`.
5. Write it down either way: `verify --attempt "idea" --points R4 --kept no --why "..."`.
6. Record a run worth keeping with `--record` and commit `results.jsonl`.

## Rules

- Never edit `goal.toml`: only the project owner changes it (CODEOWNERS).
- The full gate refuses to start without `--spend "reason"` and the owner's
  approval in `goal.toml [approval]`.
- Raw evidence never goes into git; the coding profile holds numbers only.

## Points

58 points, weights 157 scaled to 100. The A-G points are about efficiency
and general scalability at 75k (3 shards at 25,000 sessions), without the
points about growing past it (those are agents-1m's), plus:

| Group | Points | Proven by |
|---|---|---|
| R reliability | R1 leader killed mid model call, R2 lagging follower healed by snapshot, R3 follower rejoins under load, R4 random kill -9 with live moves, R5 executors killed repeatedly, R6 rebalancing mid-turn, R7 shard drained and retired, R8 topology changes under load, R9 an hour of load with a kill every 5 minutes | `scripts/agentd/prove_*.py`, `chaos.py`, `soak.py` on a local fleet (L2) |
| | R10 the gate passes 3 imported runs in a row | the results journal (L4) |
| U durability, recoverability | U1 protocol models (TLC), U2 protocol simulation, U3 snapshot memory bound | `scripts/check-tla-models.sh`, `crates/agentd/tests` (L1) |
| | U4 two zones lost, U5 point-in-time restore, U6 one-org restore | `crates/agentd-testkit/tests/recovery.rs` by test name (L1) |
| | U7 shard rebuild time, U8 leader back after losing a shard, U9 ship-first p99, U10 loss-window share | `scripts/agentd/drill.py` (L2) |
| | U11 object store outage, U12 disk full, U13 controlled restart at 98% memory | not built yet: the climb list offers them |

## Levels

| Level | What runs | Certainty |
|---|---|---|
| L0 model | arithmetic from measured per-agent costs | 0.15 |
| L1 simulation | fleet tests, protocol models and simulations, cluster simulation, realism | 0.30 |
| L2 local | the agentd harnesses and fault proofs on one machine | 0.50 |
| L4 full | the 75k gate on EKS (`saas dataplane-load`) | 1.00 |

There is no cloud slice: the full gate is cheap enough to be the only cloud
level. A run holding at least 95% of 75,000 sessions counts as full (the
gate holds about 72,500). It still needs `--spend` and the owner's approval
in `goal.toml [approval]`, and the owner's rule stands: run the gate only
when a change touches shard memory, log, snapshots, shipping, pressure,
Raft or turn admission under load.

## Importing gate runs

```bash
verify --import-run <saas dataplane-load run id> --commit <everruns sha> --record
```

Import in the order the runs happened: R10 counts the imported runs in a
row, newest first, whose gate results all pass. `results.jsonl` starts with
the seven gate runs of 2026-11-16 to 2026-11-23.
