---
title: Datasets and experiments shared with Mira
description: Export an eval's cases as Mira samples, import Mira datasets as cases, and publish Mira runs into experiments to compare them with other variants.
date: 2026-11-02
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [ui, api, cli]
links:
  docs: [/features/evals/]
  prs: [everruns/everruns#5165]
---

Eval datasets and Mira datasets now share a format. `everruns evals samples
export` writes an eval's cases as Mira samples, and `everruns evals samples
import` loads a Mira JSONL file as cases, matching them by name. Case settings
Mira has no field for travel with the sample, so a dataset makes the round trip
unchanged.

```bash
everruns evals samples export --eval-id eval_01933b5a000070008000000000000001 \
  | jq -c '.samples[]' > cases.jsonl
```

A Mira run published with an experiment name joins that experiment, one
variant per target. The experiment page compares it case by case with the
other variants and marks it as run in Mira.

See [Evals](/features/evals/).
