---
title: Framework drops the durable turns option and the workflow engine
description: Framework sessions run as leased actors that resume from the event log, so the durable feature, the workflow engine and everruns-durable-engine are removed.
date: 2026-10-12
kind: breaking
significance: notable
products: [framework]
surfaces: [code]
links:
  docs: [/framework/upgrade-notes/, /explanation/durable-execution/]
  prs: [everruns/everruns#4512, everruns/everruns#4517, everruns/everruns#4629]
---

Every `Engine` session now runs as an actor that holds the session's lease
while a turn runs, and a turn a restart cut off resumes from the session's
event log. The experimental `durable` feature, `everruns-durable`'s
`workflows` feature and the `everruns-durable-engine` crate are removed.

```toml
# Before
everruns = { version = "0.47", features = ["durable"] }
# After
everruns = { version = "0.48", features = ["local"] }
```

Remove `.backend(durable::Backend::...)` from `Engine::builder()`. The
[upgrade notes](/framework/upgrade-notes/) list each change, and
[durable execution](/explanation/durable-execution/) explains the actor model.
