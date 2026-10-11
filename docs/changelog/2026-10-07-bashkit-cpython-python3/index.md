---
title: "Bashkit runs real CPython 3.14 as python3 in a sandbox"
description: "An opt-in cpython feature makes python3 in Bashkit real CPython 3.14 in a WebAssembly sandbox, with the stdlib and HTTP through Bashkit's network policy."
date: 2026-10-07
kind: feature
significance: major
products: [bashkit]
surfaces: [code]
links:
  docs: ["https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/cpython.md"]
  prs: [everruns/bashkit#2515, everruns/bashkit#2568, everruns/bashkit#2572]
---

With the off-by-default `cpython` feature, `python` and `python3` in Bashkit are
CPython 3.14: the full language, the pure-Python standard library plus modules
such as `json`, `re`, `sqlite3` and `asyncio`, and CPython's command line (`-c`,
`-m`, files, stdin). Before, `python3` was Monty, a Python subset.

```rust
let mut bash = Bash::builder().cpython().build();
```

CPython runs as a WebAssembly guest on wasmtime, started per call from a
precompiled snapshot, with limits on time, memory, output and the VFS.
`urllib.request`, `http.client` and Bashkit's own `requests` and `httpx`
modules make HTTP calls through the same allowlist and credential injection as
`curl`.

See [CPython](https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/cpython.md), [#2515](https://github.com/everruns/bashkit/pull/2515), [#2568](https://github.com/everruns/bashkit/pull/2568) and [#2572](https://github.com/everruns/bashkit/pull/2572).
