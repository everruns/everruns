---
title: "Background jobs and pipeline stages run concurrently"
description: "Bashkit now runs cmd & jobs alongside the shell and streams pipeline stages into each other with SIGPIPE, adding jobs, wait -n, kill and related builtins."
date: 2026-10-07
kind: improvement
significance: notable
products: [bashkit]
surfaces: [code]
links:
  docs: ["https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/compatibility.md"]
  prs: [everruns/bashkit#2527, everruns/bashkit#2541]
---

`cmd &` now runs concurrently with the shell, so `sleep 1 & sleep 1 & wait`
takes one second instead of two. New `jobs`, `disown`, `fg`, `bg`, `ps`,
`pgrep` and `pkill` builtins, `wait -n`, `wait -p VAR` and jobspecs work, and
`kill` ends jobs. Live jobs are capped by
`ExecutionLimits::max_background_jobs`.

Pipeline stages that run shell code, such as loops, groups and functions, now
stream into each other through a pipe with backpressure. When a reader stops,
the writer gets SIGPIPE and exits with 141, so
`while :; do echo x; done | head -n 2` prints two lines as in bash.

Both are on by default; `BashBuilder::concurrent_jobs(false)` turns them off.

See the [compatibility scorecard](https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/compatibility.md), [#2527](https://github.com/everruns/bashkit/pull/2527) and [#2541](https://github.com/everruns/bashkit/pull/2541).
