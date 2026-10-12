---
title: "Local sessions can hand work to each other"
description: "The session_coordination capability lets a coordinator Yolop session dispatch durable tasks to opted-in local sessions and wake when they finish."
date: 2026-08-18
kind: feature
significance: major
products: [yolop]
surfaces: [cli, agent]
release: "0.16.0"
supersededBy: 2026-10-10-yolop-coordination-threads
links:
  docs: ["https://github.com/everruns/yolop#cli-essentials"]
  prs: [everruns/yolop#588, everruns/yolop#594]
---

Yolop can coordinate work across separately running sessions in the same
project. A coordinator profile discovers opted-in worker sessions, reserves one,
dispatches a durable task and is woken when the worker finishes. A worker
receives the assignment as a wake and settles it with validation notes and
artifact references.

Agents and people drive it with `yolop coordination` (`list`, `status`,
`dispatch`, `complete`, `accept`, `drain`), which acts on the live session, and
a matching `/coordination` command.

```bash
yolop coordination list --json
```

See [CLI essentials](https://github.com/everruns/yolop#cli-essentials), [#588](https://github.com/everruns/yolop/pull/588) and [#594](https://github.com/everruns/yolop/pull/594).
