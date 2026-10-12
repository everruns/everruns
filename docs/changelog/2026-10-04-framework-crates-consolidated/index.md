---
title: Framework crates consolidated into everruns-core and the facade
description: Engine, host, builtins, MCP, AG-UI, driver and contract crates merged into fewer packages, so Framework users update dependencies and imports when upgrading.
date: 2026-10-04
kind: breaking
significance: notable
products: [framework]
surfaces: [code]
release: "0.41.0"
links:
  docs: [/framework/upgrade-notes/]
  prs: [everruns/everruns#4035, everruns/everruns#4077, everruns/everruns#4081, everruns/everruns#4083, everruns/everruns#4084]
---

The published crate set shrank to 37. Each old crate shipped one deprecated
forwarding release before removal:

- Vendor driver crates moved into `everruns-drivers`, one feature per vendor
  (0.35).
- Provider, capability and model-profile APIs moved into `everruns-contracts`
  (0.37).
- `everruns-platform` became `everruns-capabilities` (0.39).
- Engine, host, builtins, MCP and AG-UI became `everruns-core` features (0.40),
  and their shims were removed in 0.41.

```toml
everruns-drivers = { version = "0.41", features = ["openai"] }
```

No capability was removed, and published shim versions are not yanked.

See the [upgrade notes](/framework/upgrade-notes/). PRs [#4035](https://github.com/everruns/everruns/pull/4035),
[#4077](https://github.com/everruns/everruns/pull/4077), [#4081](https://github.com/everruns/everruns/pull/4081), [#4083](https://github.com/everruns/everruns/pull/4083), [#4084](https://github.com/everruns/everruns/pull/4084).
