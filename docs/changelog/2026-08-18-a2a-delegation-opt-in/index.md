---
title: A2A delegation in the everruns crate is now an opt-in feature
description: Outbound A2A delegation moved behind a new a2a Cargo feature that is off by default, so local runtime builds no longer compile a second HTTP and TLS stack.
date: 2026-08-18
kind: breaking
significance: notable
products: [framework]
surfaces: [code]
release: "0.20.0"
links:
  docs: [/framework/a2a/, /capabilities/a2a-agent-delegation/]
  prs: [everruns/everruns#3221]
---

The `local` feature of the `everruns` crate no longer turns on A2A delegation. Applications that delegate to remote A2A agents must enable the new `a2a` feature:

```toml
everruns = { version = "*", features = ["local", "a2a"] }
```

Builds without it skip the A2A client and its HTTP and TLS dependencies, and the A2A delegation capability is not registered.

See [A2A](/framework/a2a/), [A2A agent delegation](/capabilities/a2a-agent-delegation/) and [#3221](https://github.com/everruns/everruns/pull/3221).
