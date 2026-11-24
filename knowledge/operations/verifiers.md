---
type: Design
title: Verifiers
description: Executable requirements that score how close the system is to a goal; a generic engine plus one self-contained folder per goal (1M and 75k agents so far).
tags:
  - operations
  - load-testing
  - data-plane-v2
---

# Verifiers

A verifier is an executable non-functional requirement: one command that
answers "does the system meet this goal?" with a score from 0 to 100, the
evidence behind it, how sure that evidence is, and the cheapest next step
that would raise it. It is built for a coding agent to climb without anyone
telling it what to do next, and so that expensive cloud runs happen only when
nothing cheaper can raise the score. The first one is
[`verifiers/agents-1m`](../../verifiers/agents-1m/README.md): can Everruns
run 1,000,000 agents at once. Research behind it, the coding-session
analysis and the first baseline: [agents-1m analysis](agents-1m-analysis.md).
The second is [`verifiers/agents-75k`](../../verifiers/agents-75k/README.md):
the 75k gate plus the reliability, durability and recoverability it must
prove (see [Reliability at 75k](#reliability-at-75k)).

## Shape

- **agentd only.** The 1M goal is about agentd, data plane v2 (shard
  nodes, executors, edge router), not the Everruns server (owner,
  2026-11-23). Points that touch the control plane measure agentd's side of
  the link: carrying on while the controller is down, streaming records out,
  issuing allowances.

- **One folder per goal, an engine that knows nothing about the goal.**
  `verifiers/lib/` is plain Python from the standard library: points,
  levels, certainty, freshness, gates, journals, the climb list, the CLI,
  and a few helpers any verifier could use (run a command, read a script's
  JSON, run cargo tests). A verifier's folder holds the rest: what its goal
  means (goal, levels, points, an optional layout), `verifier.py` (the
  hooks that give its targets their names, its extra exclusion rules, run
  imports and report sections), its checks, its journal and its tests. A
  verifier without `verifier.py` gets neutral hooks, so a functional goal
  ("every page is in Ukrainian", the engine's own test fixture) needs only
  points and a check. Owner, 2026-11-24: lib stays agnostic of what is
  verified, there is no shared layer between verifiers, and some
  duplication is fine. agents-1m (horizontal scale: cells talking to each
  other at extreme size) and agents-75k (efficiency, general scalability,
  reliability) verify different goals, so each keeps its own copy of the
  agentd checks and the coding workload. A check moves to `lib/` only when
  a verifier outside agentd would use it.
- **A locked goal.** `goal.toml` pins every number that changes the answer
  (pace per agent, peak, ramp, failures during the run, the share of coding
  agents, error and cost bars), so the score cannot rise by quietly
  redefining "agent". Its fingerprint is on every result; results under
  another fingerprint earn nothing; only the owner changes it (CODEOWNERS).
  `layout.toml`, the fleet the goal is projected onto, is a lever the
  climber may change.
- **No load tooling of its own.** Adapters call the harnesses that exist
  (`scripts/agentd/*`, the agentd fleet tests, the SaaS `dataplane-load`
  workflow) with the goal's numbers and read their output. When a harness
  changes its output, its adapter changes in the same PR; an adapter test
  feeds it a recorded output and checks the harness still prints every key
  the adapter reads.

## Score

Each verification point has a target (an expression over the goal and the
layout, so milestones and layout changes move it), a weight and a level per
result. A point's credit is closeness to target (m, partial credit, at the
unfavourable end of repeated samples) x the level's certainty (c) x
freshness (f), kept at its best level; the score is the weighted sum scaled
to 100. Rules that keep it honest:

| Rule | Why |
|---|---|
| A failed gate (lost or doubled work, replicas disagree, an out-of-memory kill) caps the score at 25; the newest gate result wins | an OOM kill on the Mac mini today outweighs a clean cloud run last week |
| Freshness halves when the files a point watches change (a digest of their contents, so squash merges do not reset it), and again after 30 days | evidence about old code is weaker |
| Per-shard points count only at the goal's density (25,000 sessions per shard); shared parts only from a run of the goal's (or a milestone's) full size | density, not size, is what lets a small run speak for 1M; shared parts do not scale out |
| In agents-75k, a shard soak below goal density still counts for per-shard points when its Raft log reached about the gate's share of the memory limit, at certainty 0.35 scaled by how often a soak's out-of-memory verdict matched the next imported gate run (`levels.toml` `[L2.scaled]`) | a 1/8 soak at snapshot 60,000 reproduced the gate's kills on the Mac mini, where a full-density soak does not fit; its own track record decides how much it is trusted |
| A run with a saturated load generator or a coding load off its profile is invalid | a slow generator makes the system look fast |
| The model level gives at most 0.15 per point | arithmetic alone tops out at 15 |

Certainties start as guesses (model 0.15, simulation 0.30, local 0.50, cloud
slice 0.80, full 1.00). Every cloud run records what the cheaper levels
predicted next to what it found; a level that keeps being wrong about a
point loses certainty.

The headline numbers next to the score: **proven agents** (the largest
cloud run with every measured gate passing) and **projected agents** (where
the scaling curve fitted to 1, 2, 4 and 8 shards stops serving the goal's
pace). Milestones at 100k, 250k and 500k score the same goal scaled down.

## The climb loop

The report ranks every action by points gained per hour: run a check at a
higher level, close a measured gap, build a missing check, refresh stale
evidence. Paid actions are listed apart and refuse to start without a
written reason; the full run also needs the owner's approval recorded in the
goal file. A climber keeps a change only when the score rose beyond noise
and nothing regressed against the last checkpoint, and journals every
attempt in `attempts.jsonl`, kept or not.

Storage: `results.jsonl` (one line per recorded run, numbers only, about
8 KB) and `attempts.jsonl` are committed; raw evidence (driver reports,
samples, heap profiles, the simulation timeline) stays in
`target/verifiers/<goal>/<time>/` or the workflow's artifact for 30 days.
Lines older than 90 days fold to their headline unless they still hold a
point's newest evidence (`verify --compact`).

## Simulation environment

The free levels need no cloud account and no real model: llmsim plays every
model call at every level (the coding agents through scenarios generated
from measured coding sessions), the agentd fleet tests run real shard,
executor and edge code in one process, and a cluster simulation projects the
whole fleet onto machines (pods with their limits, zones, the goal's ramp,
peak and zone loss), fed by per-agent costs measured at the higher levels.
It names the first line crossed on the way to the goal and what the fleet
needs in machines and money.

## Reliability at 75k

A gate run proves the fleet held its load with the failures the workflow
injects. It cannot show that agentd stays correct under the other things
that go wrong, that its restores do what they promise, or that a pass was
not luck. The 75k verifier keeps every 1M point that applies at 75k and
adds two groups (owner, 2026-11-23):

- **Reliability (R).** The existing fault proofs, run on a local fleet: a
  leader killed mid model call, a follower left behind and healed by a
  snapshot, a follower rejoining under load, random kill -9 with live
  moves, executors killed repeatedly, rebalancing mid-turn, a shard drained
  and retired, topology changes under load, and an hour of load with a
  kill every five minutes. R10 asks the gate to pass three imported runs in
  a row, so one lucky pass does not settle it.
- **Durability and recoverability (U).** The TLA+ protocol models (bug
  configurations must still fail), the protocol simulation, the snapshot
  memory bound, the restore tests (two zones lost, point in time, one org),
  and the disaster drill's numbers: rebuild time, time to a leader, the
  ship-first p99 of must-survive writes and the share of messages caught
  in the loss window. Store outage, disk full and the controlled memory
  restart have no proof yet; the climb list offers building them.

The full 75k gate is the only cloud level (no slice), and a run holding 95%
of the sessions counts as full, since the gate holds about 72,500.

## How the verifier is tested

A verifier is useful only if it fails when the system is broken. Its tests
(`verifiers/<name>/tests`, run by `.github/workflows/verifiers.yml`)
cover the scoring rules on fixed inputs, each adapter against a harness
output, a backtest that replays seven saved 75k runs and must reproduce
their real verdicts, the cluster simulation, the coding profile's privacy
(no content leaves a transcript) and realism, and the entry point end to
end. Planted-fault runs on the Mac mini (a point with no fault that fails it
is not trusted) and repeatability (three runs on unchanged code within one
point) are the next layer.
