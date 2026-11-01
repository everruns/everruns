---
type: Specification
title: "Eval experiments"
description: "Datasets with versions, experiments over variants, paired comparison, and the CI gate."
tags:
  - everruns
  - evaluation
---
# Eval experiments

## Purpose

An experiment answers "is this change better?" for an agent. It runs one
dataset at one version against one to four variants, several trials each,
and compares the variants case by case. A gate turns the answer into a pass
or a failure that CI can act on.

This is step 3 of the Evals rebuild. The UI that shows experiments (the Evals
section and an agent's Evals tab) follows the approved mockups.

## Concepts

- **Dataset**: an eval's cases. The eval's `version` counts changes to them.
  Database triggers bump it on every case insert, update and delete, because
  cases change through several paths (authoring, import, ATIF) and a missed
  bump would compare runs of different datasets as if they were one. Every
  run records the version it was created from.
- **Variant**: a label plus an optional target and model override. The
  first variant is the baseline. Omitted fields fall back to the eval's own
  target, as for any run.
- **Experiment**: the dataset version, the variants, trials per case (default
  3) and an optional spending cap. Each variant is an ordinary eval run
  tagged with the experiment and its label, so variants get durable case
  tasks, trials, budgets and the starter's authority without a second runner.
  The cap is split evenly between the variants' runs so one expensive variant
  cannot starve the others. All variants start together. When the org's
  concurrent run limit cannot fit them, the experiment is refused up front
  instead of left half started.

## Comparison

Variants ran the same cases, so they are compared in pairs, not by overall
rate alone. A case's value under a variant is the share of its finished
trials that passed. For each variant other than the baseline:

- the mean per-case difference in percentage points;
- a 95% interval from a paired bootstrap over cases (2,000 resamples, seeded
  from the experiment id so the same results always give the same interval);
- the verdict: an improvement when the whole interval is above zero, a
  regression when it is below, otherwise within noise. With fewer than three
  paired cases there is no verdict, because there is nothing to resample;
- the cases it **fixed** (it passes every trial, the baseline did not) and
  **broke** (the baseline passed every trial, it does not).

Cases whose pass rate differs from the baseline's are listed first.

## Gate

The gate judges one variant, the last one by default, against optional bars:
a minimum pass rate, a minimum share of cases passed in every trial (pass^k),
and "no regression" against the baseline. It answers 409 while a variant's
run is still going and 422 with every missed bar when it fails, so
`everruns evals experiments gate` exits non-zero and a CI step fails without
parsing JSON. A variant whose run failed or was cancelled never passes.

## Not yet

- Pinning an agent revision, or running an unpublished draft, as a variant.
  Variants change the model, harness, agent or prompt through the target today.
- "Run evals before publishing" in the agent editor, which needs the above.

## Sources of truth

- [`crates/server/src/domains/evals/experiments/`](../../crates/server/src/domains/evals/experiments)
  owns validation, creation, the comparison, the gate and the commands.
- [`crates/server/migrations/20261101142242_eval_experiments.sql`](../../crates/server/migrations/20261101142242_eval_experiments.sql)
  owns the version triggers and the experiment table.

Related: [Evals](evals.md), [Budgeting](../security/budgeting.md).
