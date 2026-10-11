---
title: "Yolop memory can be scoped to a repository"
description: "The remember, recall and forget tools in Yolop take a global or repository scope, and repository memory is shared across all linked git worktrees."
date: 2026-08-18
kind: feature
significance: notable
products: [yolop]
surfaces: [agent]
release: "0.16.0"
links:
  docs: ["https://github.com/everruns/yolop#skills-memory-and-hooks"]
  prs: [everruns/yolop#584]
---

Yolop's `remember`, `recall` and `forget` tools take a `scope` argument:
`global`, the default as before, or `repository`. Repository memory is stored in
`.yolop/memories.json` under the main repository root, so every linked worktree
of that repository reads the same notes. Global memory still follows you across
projects.

The agent picks the scope when it calls the tool, and the system prompt labels
global and repository memory separately. Any other scope value is rejected.

See [Skills, memory, and hooks](https://github.com/everruns/yolop#skills-memory-and-hooks) and [#584](https://github.com/everruns/yolop/pull/584).
