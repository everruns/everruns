---
title: "Bashkit awk is rewritten to follow gawk 5.2"
description: "Bashkit's awk, gawk, mawk and nawk builtins now target gawk 5.2, with POSIX awk, common gawk extensions, and system() and pipes through the sandbox shell."
date: 2026-10-07
kind: improvement
significance: notable
products: [bashkit]
surfaces: [code]
links:
  docs: ["https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/compatibility.md"]
  prs: [everruns/bashkit#2583, everruns/bashkit#2561]
---

Bashkit's `awk` was rewritten to target gawk 5.2, Debian's `awk`. It covers
POSIX awk plus the gawk extensions scripts commonly use; `system()`, output
pipes and `cmd | getline` run through the sandbox shell. Number output,
`printf`, the `rand()` sequence, leftmost-longest regex matching, diagnostics
and options follow gawk. `gawk`, `mawk` and `nawk` run the same builtin.

On Bashkit's Debian-oracle awk tests, the pass rate went from 51.5% to 93.8%.
Resource caps still apply, and `split` and record splitting count against the
memory cap.

See the [compatibility scorecard](https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/compatibility.md), [#2583](https://github.com/everruns/bashkit/pull/2583) and [#2561](https://github.com/everruns/bashkit/pull/2561).
