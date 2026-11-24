---
title: Yolop retains coding evidence and accepts live corrections
description: Yolop keeps recent check outcomes and investigation context across history trimming, and receives corrections while an agent task is running.
date: 2026-11-23
kind: improvement
significance: notable
products: [yolop]
surfaces: [cli]
links:
  docs: [https://github.com/everruns/yolop/blob/main/docs/features/tools.md]
  prs: [everruns/yolop#965]
---

Yolop retains recent check outcomes and the latest accepted investigation
checkpoint when history is trimmed or a session is reopened. Checks become stale
after a later observed edit or potentially mutating shell command, so the agent
can distinguish earlier results from checks of the current code. This tracks
Yolop's tool calls, not external editors or services.

In the terminal, type a correction and press Enter while the agent is working:

```text
Preserve the public API and rerun the affected tests after the final edit.
```

The correction joins the active task when it can still accept input. Otherwise,
it starts a follow-up task without being submitted twice. Press Escape twice to
cancel the active task. See [Yolop's coding tools](https://github.com/everruns/yolop/blob/main/docs/features/tools.md).
