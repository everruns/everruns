# agents-1m: can Everruns run 1,000,000 agents at once?

The target is agentd (data plane v2: shard nodes, executors, edge router)
only. The Everruns server is out of scope; tests stand in for it with the
testkit's fake control plane.

```bash
validators/agents-1m/validate --plan     # what to do next (seconds, runs nothing)
validators/agents-1m/validate --quick    # model + simulation levels, the report (seconds)
validators/agents-1m/validate            # the same plus the cargo fleet tests (~25 min)
```

Python 3.11+ standard library only. The report lands in
`target/validators/agents-1m/<time>/report.md` (and `report.json` for
agents) and is printed. `just validate agents-1m <args>` is the same.

## Climbing the score

1. `validate --plan`: the climb list, ranked by points gained per hour.
   Paid actions are listed apart.
2. Take the top free item. "run X" runs a check at a higher level;
   "close the gap" fixes a measured shortfall in the code; "build the check"
   adds a check for a point nothing measures yet (see `../lib/checks/__init__.py`
   `NOT_BUILT`, then move it into a module with an adapter test).
3. Re-run only what you touched: `validate --level local --points C1`.
4. Keep the change only if the score rose by more than the noise and no
   gate fell: `validate --checkpoint before` first, `validate --against before`
   after (exit 1 on a regression).
5. Write it down either way, so nobody retries a dead idea:
   `validate --attempt "smaller snapshot buffers" --points C1 --before 23.7 --after 24.1 --kept yes --why "..."`
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
| L0 model | arithmetic from measured per-agent costs, scaling-curve fit | 0.15 |
| L1 simulation | fleet tests with llmsim, cluster simulation of the whole fleet, realism check | 0.30 |
| L2 local | the agentd harnesses at one shard of 1M density (about 30 GB: the Mac mini) | 0.50 |
| L3 cloud slice | EKS at goal density, 125,000 agents | 0.80 |
| L4 full | EKS, 1M agents | 1.00 |

A point's credit is closeness to target x certainty x freshness, at its best
level; freshness halves when the files it watches change and again after 30
days. A failed gate (A1-A4) caps the score at 25. Per-shard points count
only at goal density; shared points only from a run of the goal's size.

## Files

| Path | What |
|---|---|
| `validate` | the entry point (`../lib/engine/cli.py`, shared with every validator) |
| `goal.toml` | what 1M means (owner only) |
| `layout.toml` | the fleet the goal is projected onto (a lever) |
| `points.toml`, `levels.toml` | the 38 points and their targets; level certainties |
| `../lib/` | the shared engine, checks and coding workload (see `validators/README.md`) |
| `results.jsonl`, `attempts.jsonl`, `checkpoints/` | the committed climbing record |
| `tests/` | `python3 -m unittest discover -s tests -t .` (CI: `.github/workflows/validators.yml`) |

## Refreshing the coding profile

```bash
../lib/workload/capture.py ~/.claude/projects --out ../lib/workload/coding-profile.json
../lib/workload/generate.py      # rewrites crates/drivers/llmsim/scenarios/coding-profile.json
```

Review the diff like a goal change. The profile today comes from very few
sessions; a run over the Mac mini's Claude Code transcripts is the cheapest
way to make G1-G6 realistic.

## Importing a cloud run

```bash
validate --import-run 45197793040 --commit <everruns sha of the image> --record
```

`--import-run` takes a saas `dataplane-load` run id (downloads its artifact
with `gh`) or an unpacked artifact folder.
