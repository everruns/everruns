---
title: Control Yolop continuation budgets and trace stop reasons
description: "Yolop supports five automatic continuations, configurable limits, durable stop and retry reasons, and OpenTelemetry budget diagnostics."
date: 2026-10-16
kind: improvement
significance: notable
products: [yolop]
surfaces: [cli, operations]
links:
  docs: ["https://github.com/everruns/yolop/blob/main/docs/features/tools.md#plans-and-completion"]
  prs: [everruns/yolop#858]
---

Yolop allows five automatic continuation turns by default, with configurable
turn, token, and elapsed-time limits. Ask Yolop to change the budget or use:

```bash
yolop config set continuation_max_turns 10
yolop config set continuation_max_tokens 512000
yolop config set continuation_max_seconds 3600
```

The first reached limit pauses work and asks for approval to resume with a fresh
budget, without prescribing reply words. Named profiles can override the limits;
attached changes apply at the next continuation check without resetting usage.

Decisions, retry reasons, budget usage, and exhausted limits survive restarts in
private session logs. Find earlier stops with:

```bash
yolop sessions search --query "continuation exhausted"
```

Enabled trace extensions receive the same diagnostics. The Logfire exporter
emits OpenTelemetry budget spans with decision attributes correlated to the
completed turn. See
[Plans and completion](https://github.com/everruns/yolop/blob/main/docs/features/tools.md#plans-and-completion).
