---
title: Compare agent variants with eval experiments
description: Run one eval dataset against several models side by side, see which cases each fixed or broke, and gate CI on a verdict that accounts for run-to-run noise.
date: 2026-11-01
kind: feature
significance: major
products: [platform, cloud]
surfaces: [ui, api, cli]
links:
  docs: [/features/evals/]
  prs: [everruns/everruns#5154]
---

**In the UI:** Quality → Evals now opens on an overview of every agent with a
dataset, its pass rate and trend, and the findings that need a look. **Run
experiment** runs one dataset against two to four models with several trials
per case. The experiment page lists the cases each variant fixed or broke and
says whether the change is likely real or could be noise. Each agent page has
an Evals tab.

![An experiment comparing GPT-6 Luna and GPT-6.1 Sol on the same six cases, with pass rate, cost per case and a verdict](./experiment@2x.png)

**In code:** an eval's cases now carry a version, and the same experiment is
one CLI call, with a gate that fails a CI step when a variant misses a bar:

```bash
everruns evals experiments gate --eval-id eval_01h9 --experiment-id evalexp_01h9 \
  --min-pass-rate 0.9 --no-regression true
```

See [Evals](/features/evals/).
