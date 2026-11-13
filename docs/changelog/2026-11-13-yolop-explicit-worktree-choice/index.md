---
title: "Choose a worktree before the first edit"
description: "New Yolop sessions in auto worktree mode require a choice before file edits or shell commands, while read-only tools remain available."
date: 2026-11-13
kind: improvement
significance: notable
products: [yolop]
surfaces: [cli, agent]
links:
  docs: ["https://github.com/everruns/yolop/blob/main/knowledge/specs/worktrees.md"]
  prs: [everruns/yolop#944]
---

New Yolop sessions in `auto` worktree mode block file edits and shell commands
until the agent explicitly chooses an isolated worktree or the current checkout. Read-only
tools can still inspect the repository before that decision.

To work in isolation, the agent runs:

```bash
yolop worktree init
```

To deliberately keep working in the current checkout instead:

```bash
yolop worktree stay
```

While the choice is pending, only standalone `yolop worktree` commands can run
through the shell. Leading and trailing whitespace are accepted, but combining
a worktree command with another shell command is rejected. The choice is enforced
rather than depending on the agent following a prompt. Previously saved sessions
keep their saved capabilities; this does not activate the gate retroactively.

See [Worktree modes](https://github.com/everruns/yolop/blob/main/knowledge/specs/worktrees.md).
