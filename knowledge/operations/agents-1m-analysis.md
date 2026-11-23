---
type: Investigation
title: agents-1m analysis
description: Evidence behind the 1M agents validator - what 1M means, the 75k backtest, the coding-session analysis and the first baseline.
tags:
  - operations
  - load-testing
  - data-plane-v2
---

# agents-1m analysis

Research and first results behind the [1M agents validator](validators.md)
([`validators/agents-1m`](../../validators/agents-1m/README.md)). The design
page says how the validator works; this page records why its numbers are what
they are, and what the first runs found.

## What "1M agents" means

One million live sessions at the pace the 75k gate already ran: one turn
every 6 minutes per agent, about 2,800 turns a second steady, with a 3x peak
for 10 minutes, a 30 minute ramp, an hour of steady load and failures during
the run (a node per shard group, a quarter of the executors, one zone for 5
minutes). A quarter of the agents are long-running coding agents (owner,
2026-11-23), shaped from this project's own coding sessions. Many small orgs
plus one org of 100,000 agents. The goal and its fingerprint live in
`goal.toml`; only the owner changes it.

Two limits moved from the first draft when real data arrived:

- **Ship lag p99 15 s, not 2 s.** The passing 75k runs measure the p99 of
  records not yet shipped to object storage at 8 to 11 s while the store is
  healthy. 2 s would fail every run we consider good, so the bar is set just
  above what passing runs show and tightens later.
- **Cost bar includes store writes.** Run 45197793040 spent about
  $3.3e-05 per agent-hour on compute and $7.0e-05 on object-store writes. The
  bar is the sum, $1.03e-04. At 1M the model puts store writes at about
  $67/h beside about $87/h of machines, so write batching is a cost lever as
  large as packing.

## Backtest on the 75k runs

The import adapter scores recorded EKS runs without re-running them. Seven
runs are kept as fixtures (`tests/fixtures/runs/`), and the verdicts match
what the runs were judged at the time:

| Run | Verdict | Why |
|---|---|---|
| 43774871232, 43803384547, 43827766384 | fail | lost or repeated messages (A1), out-of-memory kills (A4), errors (E1) |
| 44355318472, 45179434646 | fail | out-of-memory kills only (A4) |
| 44396952768, 45197793040 | pass | gate clean |

The 75k runs cannot measure F1 (turn time drift over a steady hour): their
sessions stop after 10 turns, so the load falls off and the slope is always
negative. The import leaves F1 unmeasured with a note rather than credit a
fake pass. D7 (the load generator itself was healthy) is also unmeasured
there, because the drivers ran without a CPU limit.

## Coding sessions

`workload/capture.py` reads Claude Code session records and keeps numbers
only (durations, token counts, tool-call counts and kinds), never content.
The first profile is thin: 2 sessions and 15 turns, this project's
validator session and a bashkit issue session. What it shows already:

- A coding turn makes a median of 5 model calls (p90 about 22), against 3 for
  the general mix.
- Context per call is large: median about 246k tokens, p90 about 354k. Memory
  per coding session is therefore far above a chat session; the model assumes
  4x until G1 measures it.
- Tool output is small per call (median about 1 KB, p99 about 35 KB); shell
  and MCP calls dominate, edits are rare in the record.
- Gaps between turns are short (median about 1 minute), so coding agents are
  much hotter than the 6 minute pace while they work.

`workload/generate.py` turns the profile into 11 llmsim scenarios
(`coding-p01` to `coding-p99`); the realism check (G6) fails the run when they
drift more than 10% from the profile. The sample should be refreshed with many
more sessions, ideally on the Mac mini lab, before G1 and G2 are trusted.

## First baseline

The first recorded score is about 28 of 100, with 72,537 agents proven (the
best passing 75k run, at cloud-slice certainty). The L1 fleet tests pass:
exactly once (A1), tools at most once (A2), replicas agree (A3), streams
(B5), executor loss (E4) and control plane down (E5). The model sizes 1M agents at
40 shards, 120 node pods and about 363 executors on 42 machines of
m6a.12xlarge.

The cluster simulation finds the layout does not fit yet (D8 1.97 against a
target of 1): node memory reaches 132% and node CPU 138% at the peak. Both
come mostly from the assumed coding factors (4x memory, extra CPU per coding
turn), so the first real limit to climb is measuring them (G1, G2), not
tuning the layout. Two measured gaps also stand out: resident memory share
C3 is 0.81 against 0.15 (the memory design work in progress), and node
memory per 1,000 sessions C1 is 431 MB against 401.

The cheapest next actions the climb list names are the local scale run for
the USL fit (D1, which also unlocks the projected-agents number), the local
disaster drill (E2) and a one-hour shard soak (F1).

## Open questions

- A 125k cloud slice needs 5 shards x 3 nodes x 14 GiB = 210 GiB of node
  memory, more than the single 183 GiB node the current load workflow uses.
  The slice needs a second node or smaller node pods.
- Points marked not built (B2-B4, D2-D4, D6, E3, E6, F2, G1-G5) each need a
  driver or test that does not exist yet; the climb list ranks them by gain.
