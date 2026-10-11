---
title: Framework workspaces gain multiple isolated Git heads
description: Framework sessions can bind to one named head of a workspace, such as a local Git worktree, with fork, archive and destroy as explicit operations.
date: 2026-08-14
kind: feature
significance: notable
products: [framework]
surfaces: [code]
release: "0.18.0"
links:
  docs: [/framework/workspaces-and-environments/]
  prs: [everruns/everruns#3166]
---

The `everruns` crate now has public workspace and environment types, so several sessions can work on one repository without sharing files. A workspace has named heads; each session binds permanently to exactly one head and resumes on that same head.

With the `local` feature, a local Git-worktree provider creates each head as a worktree from a revision. Heads are isolated by default; opt into a shared head explicitly. Fork, checkpoint, archive and destroy are explicit calls, and dropping a value never deletes a worktree. The provider interface is open for other backends.

See [Workspaces and environments](/framework/workspaces-and-environments/) and [#3166](https://github.com/everruns/everruns/pull/3166).
