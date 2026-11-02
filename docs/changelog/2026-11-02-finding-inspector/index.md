---
title: The Inspector explains findings and groups similar ones
description: Serious findings now come with a plain account of what happened and a next step, similar findings show as one row, and findings can be dismissed with a reason.
date: 2026-11-02
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api, cli]
links:
  docs: [/features/evals/]
  prs: []
---

When a classifier flags a session as stuck, abusive or following injected
instructions, or a schedule runs away, the **Inspector** now reads the session
and writes up what happened, how serious it is and what to do next, on your
organization's default model. Each finding opens its own page under
**Evals → Findings** with the detector's numbers and that account, and similar
findings are grouped so ten copies of one problem show as one row.

Findings that need nothing can be dismissed with a reason, and stay closed when
the same session is flagged again:

```bash
everruns health-issues dismiss --issue-id 550e8400-e29b-41d4-a716-446655440000 \
  --note 'Expected: nightly load test'
```

Deployment operators can review suspected abuse across organizations with
`everruns system detections`. See [Evals](/features/evals/).
