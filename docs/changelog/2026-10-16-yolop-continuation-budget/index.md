---
title: Configure automatic continuation budgets in Yolop
description: "Yolop allows five automatic continuations by default, configurable turn, token and time limits, and conversational approval to resume with a fresh budget."
date: 2026-10-16
kind: improvement
significance: notable
products: [yolop]
surfaces: [cli, operations]
links:
  docs: ["https://github.com/everruns/yolop/blob/main/docs/features/tools.md#plans-and-completion"]
  prs: [everruns/yolop#858]
---

Yolop now allows five automatic continuation turns by default, with configurable
turn, token, and elapsed-time limits. Ask Yolop to change the budget or use the
existing config commands:

```bash
yolop config set continuation_max_turns 10
yolop config set continuation_max_tokens 512000
yolop config set continuation_max_seconds 3600
```

The first reached limit pauses work. Yolop names that limit and asks you to
approve resuming with a fresh budget, without requiring specific reply words.
Approval refreshes the budget without increasing the saved limits.

Named profiles can override the limits. Attached configuration changes apply at
the next continuation check and retain usage already consumed. Clear a setting
to restore its inherited value or default. See
[Plans and completion](https://github.com/everruns/yolop/blob/main/docs/features/tools.md#plans-and-completion).
