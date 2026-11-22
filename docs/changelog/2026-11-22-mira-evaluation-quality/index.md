---
title: Mira checks evaluation coverage before CI passes
description: Mira separates missing grading evidence from target quality, compares candidates with saved baselines, and calibrates judges against reviewed labels.
date: 2026-11-22
kind: feature
significance: notable
products: [mira]
surfaces: [cli, code]
links:
  docs: [https://github.com/everruns/mira/blob/main/docs/evaluation-quality.md]
  prs: [everruns/mira#95]
---

Mira can now require grading evidence before an evaluation passes CI, compare
candidates with a saved baseline, and check judges against reviewed labels.

```bash
mira run --study study.rs --min-coverage 1 --require-scorer 'llm_judge:*'
mira compare BASELINE CANDIDATE --max-regressions 0
```

Saved runs retain full execution artifacts and original sample references for
rescoring. Rust, Python, and TypeScript studies preserve trial identities and
seeds. Missing evidence stays separate from target failures across every run
report format. Trajectory-aware judges receive tool arguments and observations;
failed cases can become review records for regression datasets.

The [evaluation-quality guide](https://github.com/everruns/mira/blob/main/docs/evaluation-quality.md)
includes calibration, provenance, and a complete offline workflow example.
