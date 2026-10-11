---
title: "Bashkit adds a make builtin and 21 everyday commands"
description: "Bashkit gains a GNU make 4.3 subset that runs recipes in the sandbox shell, plus nohup, flock, getconf, free and 17 other commands that scripts often call."
date: 2026-10-07
kind: feature
significance: notable
products: [bashkit]
surfaces: [code]
links:
  docs: ["https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/compatibility.md"]
  prs: [everruns/bashkit#2551, everruns/bashkit#2546]
---

`make` is now a builtin: a GNU make 4.3 subset with every assignment flavor,
conditionals, `define`, `include`, explicit, pattern and suffix rules,
functions such as `foreach`, `call`, `wildcard` and `shell`, and the options
`-f -C -n -k -q -s -i -B -e -j` (`-j` runs sequentially). Recipes run through
the sandbox shell, and messages and exit codes follow GNU make.

Twenty-one commands that used to report "command not found" are added: `arch`,
`sum`, `shasum`, `egrep`, `fgrep`, `link`, `unlink`, `chgrp`, `nohup`, `nice`,
`flock`, `getconf`, `tty`, `sync`, `hostid`, `groups`, `logname`, `users`,
`who`, `uptime` and `free`. `cmd N>file` with N of 3 or more now opens only
fd N.

See the [compatibility scorecard](https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/compatibility.md), [#2551](https://github.com/everruns/bashkit/pull/2551) and [#2546](https://github.com/everruns/bashkit/pull/2546).
