# agents-1m: can Everruns run 1,000,000 agents at once?

This verifier asks whether cells work together at extreme size: 40 shards
(and later hundreds) behaving as one system at 1M sessions. It scores
scaling out, cells working together (the directory and edge router, routing
a turn to its cell, sessions moving between cells, rebalancing, membership,
subagents on another cell), the control plane link at 1M rate, fleet-wide
failures (a zone lost, rolling upgrades) and whether the fleet fits real
machines. Per-agent efficiency (memory, CPU, store writes and cost per
agent, long and coding sessions) is agents-75k's goal: its measurements
feed this verifier's model and cluster simulation and score nothing here.

The target is agentd (data plane v2: shard nodes, executors, edge router)
only. The Everruns server is out of scope; tests stand in for it with the
testkit's fake control plane.

```bash
verifiers/agents-1m/verify --plan     # what to do next (seconds, runs nothing)
verifiers/agents-1m/verify --quick    # model + simulation levels, the report (seconds)
verifiers/agents-1m/verify            # the same plus the cargo fleet tests (~25 min)
```

Python 3.11+ standard library only. The report lands in
`target/verifiers/agents-1m/<time>/report.md` (and `report.json` for
agents) and is printed. `just verify agents-1m <args>` is the same.

## Climbing the score

1. `verify --plan`: the climb list, ranked by points gained per hour.
   Paid actions are listed apart.
2. Take the top free item. "run X" runs a check at a higher level;
   "close the gap" fixes a measured shortfall in the code; "build the check"
   adds a check for a point nothing measures yet (see `checks/__init__.py`
   `NOT_BUILT`, then move it into a module with an adapter test).
3. Re-run only what you touched: `verify --level local --points D1`.
4. Keep the change only if the score rose by more than the noise and no
   gate fell: `verify --checkpoint before` first, `verify --against before`
   after (exit 1 on a regression).
5. Write it down either way, so nobody retries a dead idea:
   `verify --attempt "batch directory updates" --points X6 --before 12.1 --after 13.0 --kept yes --why "..."`
6. Record a run worth keeping with `--record` and commit `results.jsonl`
   with the change it measured.

Read before acting: this file, the latest `report.md`, `attempts.jsonl`,
then only the harness behind the action you picked.

## Rules

- Never edit `goal.toml`: it defines what the score means, its fingerprint
  is on every result, and only the project owner changes it (CODEOWNERS).
  `layout.toml` (machines, pod sizes, idle limit) is a lever you may change.
- Paid levels refuse to start without `--spend "reason"`. The cloud slice
  runs only when the climb list puts it first and local checks pass, at most
  weekly; the full run needs the owner's approval in `goal.toml [approval]`.
- Raw evidence never goes into git: only `results.jsonl` (about 8 KB a run),
  `attempts.jsonl` and `checkpoints/`.
- The coding profile holds numbers only. Never commit transcripts.

## Levels

| Level | What runs | Certainty |
|---|---|---|
| L0 model | arithmetic from per-agent costs (measured by agents-75k), scaling-curve fit | 0.15 |
| L1 simulation | fleet tests with llmsim, cluster simulation of the whole fleet | 0.30 |
| L2 local | the agentd harnesses at one shard of 1M density (about 30 GB: the Mac mini) | 0.50 |
| L3 cloud slice | EKS at goal density, 125,000 agents | 0.80 |
| L4 full | EKS, 1M agents | 1.00 |

A point's credit is closeness to target x certainty x freshness, at its best
level; freshness halves when the files it watches change and again after 30
days. A failed gate (A1-A4) caps the score at 25. Per-shard points count
only at goal density; shared points only from a run of the goal's size.

## Reading this verifier

In this order; nothing outside this folder is needed except the engine.

| File | What |
|---|---|
| `README.md` | this page: the goal, how to climb, the rules |
| `goal.toml` | what 1M means: agents, pace, density, failures, limits (owner only, fingerprinted) |
| `points.toml` | the 23 points: target, weight, gate or validity, the files each watches |
| `levels.toml` | the five levels and their certainty |
| `layout.toml` | the fleet the goal is projected onto (a lever) |
| `verifier.py` | what this verifier adds to the engine: agents and shards as target names, the density and run-size rules, proven and projected agents, run import and the paid dispatch, its report sections |
| `checks/` | one module per kind of evidence: `model`, `cluster`, `fleettests`, `local`, `cloud`; `__init__.py` lists them and the points not built yet |
| `workload/` | the coding profile (numbers only) and the llmsim scenarios generated from it |
| `results.jsonl`, `attempts.jsonl`, `checkpoints/` | the committed climbing record |
| `tests/`, `verify` | `python3 -m unittest discover -s tests -t .` (CI: `.github/workflows/verifiers.yml`), and the entry point |

The engine in `../lib/` (scoring, freshness, report, climb list, CLI) knows
nothing about agents; this folder is the whole of what 1M means. agents-75k
has its own copy of the agentd checks, because its goal is a different one.

## Refreshing the coding profile

```bash
workload/capture.py ~/.claude/projects --out workload/coding-profile.json
workload/generate.py      # rewrites crates/drivers/llmsim/scenarios/coding-profile.json
```

Review the diff like a goal change. Here the profile only shapes the load
the model projects (a quarter of the agents are coding agents); agents-75k
scores the coding sessions themselves (G1-G6) and checks the scenarios
against the profile.

## Importing a cloud run

```bash
verify --import-run 45197793040 --commit <everruns sha of the image> --record
```

`--import-run` takes a saas `dataplane-load` run id (downloads its artifact
with `gh`) or an unpacked artifact folder.
