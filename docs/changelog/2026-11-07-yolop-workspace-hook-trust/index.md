---
title: Yolop asks before running a repository's hooks
description: A workspace .agents/hooks.json no longer runs on its own. Yolop names the skipped file at startup and runs its hooks only after you trust it.
date: 2026-11-07
kind: breaking
significance: minor
products: [yolop]
surfaces: [cli]
links:
  prs: [everruns/yolop#921]
---

A repository's `.agents/hooks.json` used to run as soon as Yolop started in that
folder, including `session_start` commands with your API keys in the
environment. Yolop now ignores the file until you review it and trust it from
your own terminal:

```sh
yolop config hooks trust
```

Trust is pinned to the file's exact contents, so any change turns its hooks off
again until you re-run the command. A running session cannot grant trust.
When Yolop skips a file it says so once at startup, and
`yolop config hooks list` shows it. Your global hooks are unchanged.
