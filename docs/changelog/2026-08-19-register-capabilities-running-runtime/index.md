---
title: Register a capability on a running Framework runtime
description: A host holding a shared InProcessRuntime can add a capability discovered after startup and activate it in live sessions without rebuilding the runtime.
date: 2026-08-19
kind: feature
significance: notable
products: [framework]
surfaces: [code]
release: "0.20.0"
links:
  docs: [/framework/advanced-capabilities/]
  prs: [everruns/everruns#3224]
---

An `InProcessRuntime` can now accept capabilities after composition, for example an extension a user installs mid-conversation:

```rust
runtime.register_capability(Arc::new(capability))?;
runtime.is_capability_registered("ext:name");
```

Registration only makes the id resolvable. Sessions still enable it with `activate_capability`, so sessions that never activate it see no change. A duplicate id or a colliding alias is rejected and the existing capability stays. Readers always see a consistent registry snapshot.

See [Advanced capabilities](/framework/advanced-capabilities/) and [#3224](https://github.com/everruns/everruns/pull/3224).
