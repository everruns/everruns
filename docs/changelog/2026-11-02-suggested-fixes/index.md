---
title: Suggested fixes for findings, checked by evals
description: The Inspector suggests an instruction change for a finding; check it against the agent's evals and approve it as a new agent revision.
date: 2026-11-02
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [ui, api, cli]
links:
  docs: [/features/evals/]
  prs: [everruns/everruns#5167]
---

When an agent gets stuck, follows an injected instruction, or keeps a schedule
running with nothing to show for it, the Inspector's write-up can now come with
a suggested fix: a short addition to the agent's instructions, shown on the
finding page.

Nothing changes the agent until you act. **Check against evals** runs the
agent's eval with and without the addition as an experiment. When the check
shows no case broke, **Approve** adds the text as a new agent revision, which
you can restore from the agent's history. You can edit the text first, or
reject it with a note.

```bash
everruns evals fixes check --fix-id fix_0193... --max-cost-usd 2 --reason 'Before applying the stop rule'
everruns evals fixes apply --fix-id fix_0193... --reason 'Stop rule checked on the claims dataset'
```

See [Evals](/features/evals/).
