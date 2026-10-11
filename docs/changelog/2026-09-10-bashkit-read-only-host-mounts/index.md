---
title: "Bashkit mounts host directories read-only and over the C ABI"
description: "Bashkit's Python and Node bindings can mount a host directory read-only, and the C API gains allowlisted host-directory mounts on a running session."
date: 2026-09-10
kind: feature
significance: notable
products: [bashkit]
surfaces: [code]
release: "0.18.0"
links:
  docs: ["https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/live_mounts.md"]
  prs: [everruns/bashkit#2393, everruns/bashkit#2371]
---

**Read-only mounts.** Python `Bash.mount()` and `BashTool.mount()` take
`read_only=True`, and Node `mount()` takes `readOnly`. Reads work; writes,
`cp`, `mv`, `mkdir`, `rm` and `chmod` inside the mounted tree fail, and
`reset()` keeps the protection.

```python
bash.mount("/corpus", corpus, read_only=True)
```

**C ABI mounts.** The C API can mount real host directories, through `mounts`
in the config or `bashkit_mount` and `bashkit_unmount` on a running session.
Each root must resolve under an `allowed_mount_paths` prefix and pass the
sensitive-path denylist. With no allowlist, every mount is rejected.

See [Live mounts](https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/live_mounts.md), [C API](https://github.com/everruns/bashkit/blob/main/docs/c-api.md), [#2393](https://github.com/everruns/bashkit/pull/2393) and [#2371](https://github.com/everruns/bashkit/pull/2371).
