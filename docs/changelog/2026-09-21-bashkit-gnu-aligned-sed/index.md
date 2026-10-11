---
title: "sed is rewritten to follow GNU sed 4.9"
description: "Bashkit rewrote its sed after an audit found 46 differences from GNU sed 4.9, adding missing commands, GNU address ranges, case conversion and atomic -i."
date: 2026-09-21
kind: improvement
significance: notable
products: [bashkit]
surfaces: [code]
release: "0.18.2"
links:
  docs: ["https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/compatibility.md"]
  prs: [everruns/bashkit#2436, everruns/bashkit#2435]
---

Bashkit's `sed` was rewritten after a comparison with GNU sed 4.9 found 46
mismatches. It adds the missing commands `y = l n N P D T z F r R w W`, GNU
range-address semantics, `\U \L \u \l \E` case conversion, and real option
parsing for `-s`, `-z`, `-f`, `-l` and attached `-i.bak`. A missing final
newline is preserved, and `-i` now writes files atomically.

The same release builds VFS paths with forward slashes on every host, including
Windows.

See the [compatibility scorecard](https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/compatibility.md), [#2436](https://github.com/everruns/bashkit/pull/2436) and [#2435](https://github.com/everruns/bashkit/pull/2435).
