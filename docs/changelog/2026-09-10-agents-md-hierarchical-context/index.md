---
title: AGENTS.md files resolve hierarchically as conversation context
description: Agent instructions now load AGENTS.md from the workspace root down to the working directory and place them below harness safety instructions.
date: 2026-09-10
kind: improvement
significance: notable
products: [platform, cloud, framework]
surfaces: [agent]
release: "0.25.0"
links:
  docs: [/capabilities/agent-instructions/, /how-to/use-agents-md/]
  prs: [everruns/everruns#3501]
---

`AGENTS.md` content no longer goes into the system prompt. The agent instructions
capability resolves the configured files hierarchically, from the filesystem root
to the working directory, with deeper files winning and sibling directories out of
scope. The result is inserted as the leading user-role message of each turn,
behind a header that marks it as workspace content.

This keeps untrusted repository instructions below harness safety instructions
and keeps the system prompt stable for caching. Resolution is bounded to 16 files,
32 KiB each and 128 KiB in total.

See [Agent instructions](/capabilities/agent-instructions/) and
[Use AGENTS.md](/how-to/use-agents-md/). [#3501](https://github.com/everruns/everruns/pull/3501)
