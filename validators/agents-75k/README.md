# agents-75k: does Everruns hold 75,000 agents, and survive what goes wrong?

The 75k gate as a validator. It scores the same things the
[1M validator](../agents-1m/README.md) does, at 75k (3 shards at 25,000
sessions each), and adds what a gate run alone cannot show: that agentd
stays correct when nodes, executors, shards, zones and the store fail, that
it restores what it promises to restore, and that the gate passes run after
run. Target: agentd (data plane v2) only; the Everruns server is out of
scope.

```bash
validators/agents-75k/validate --plan     # what to do next (seconds, runs nothing)
validators/agents-75k/validate --quick    # model + simulation levels (seconds)
validators/agents-75k/validate            # plus fleet tests and protocol models (~30 min)
validators/agents-75k/validate --level local --points R4   # one fault proof
```

The climbing loop, the rules and the file layout are the 1M validator's;
read [its README](../agents-1m/README.md) for them. What differs:

## Points

58 points, weights 157 scaled to 100. The 1M points minus the ones about
growing past 75k (D1 scaling curve, D4 1M-session directory, D6 hundreds of
shards), plus:

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
validate --import-run <saas dataplane-load run id> --commit <everruns sha> --record
```

Import in the order the runs happened: R10 counts the imported runs in a
row, newest first, whose gate results all pass. `results.jsonl` starts with
the seven gate runs of 2026-11-16 to 2026-11-23.
