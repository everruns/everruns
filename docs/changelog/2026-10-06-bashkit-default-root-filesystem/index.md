---
title: "Bashkit sessions start with a default root filesystem"
description: "Every Bashkit session now has read-only /etc, /proc and /usr/bin entries, so scripts that read os-release or cpuinfo or call /usr/bin/env find what they expect."
date: 2026-10-06
kind: feature
significance: notable
products: [bashkit]
surfaces: [code]
links:
  docs: ["https://github.com/everruns/bashkit/blob/main/docs/filesystem.md"]
  prs: [everruns/bashkit#2525]
---

Every `Bash` session now starts with a synthetic root filesystem: a read-only
`/etc` (`os-release`, `passwd`, `group`, `hostname`, `hosts`, `shells`,
`timezone`), static `/proc` files (`cpuinfo`, `meminfo`, `version`, `loadavg`),
`/root` and `/dev/zero`. `/bin` and `/usr/bin` list the builtins, so a
`/usr/bin/env` shebang runs the builtin and `type`, `which` and `command -v` report
`/usr/bin/NAME`.

Session files take precedence, and the layer uses no quota and is never
snapshotted. Turn it off with:

```rust
let bash = Bash::builder().rootfs(false).build();
```

See [Filesystem](https://github.com/everruns/bashkit/blob/main/docs/filesystem.md) and [#2525](https://github.com/everruns/bashkit/pull/2525).
