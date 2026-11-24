---
title: Review changes and resume work in Yolop
description: Yolop adds a live changes panel, visual rewind, session search, inspectable prompt attachments, recovery actions, and searchable terminal commands.
date: 2026-11-24
kind: feature
significance: notable
products: [yolop]
surfaces: [cli, ui]
links:
  prs: [everruns/yolop#964]
  docs: [https://github.com/everruns/yolop/blob/main/docs/features/terminal-workflows.md]
---

Review changed files without leaving the conversation, preview a checkpoint
before restoring it, and find earlier sessions by title, repository, branch,
or recorded messages. Session drafts and attachments are restored when you return.

![Yolop changes panel with changed files and a navigable diff](./changes@2x.png)

```text
/diff
/attach src/main.rs#20-45
/timeline
```

Press **Ctrl+P** for searchable actions. The changes panel refreshes while you
work; attach a file diff or hunk to your next question. Inspect prompt context
with `/attachments`, edit longer prompts through **Ctrl+G**, and use `/recover`
after a failed turn. Approval prompts explain one-time and session-wide scope,
and `/context` separates context occupancy from cumulative usage.

See [Terminal workflows](https://github.com/everruns/yolop/blob/main/docs/features/terminal-workflows.md).
