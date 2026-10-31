---
title: Agent background work sends fewer streaming updates
description: Explicit agents and subagents retain completed notes while avoiding unused streaming updates, with native provider completion where supported.
date: 2026-10-28
kind: improvement
significance: minor
products: [platform, cloud, framework]
release: "0.50.0"
surfaces: [api, code, operations]
links:
  docs: [/features/explicit-communication/]
---

Agents using [explicit communication](/features/explicit-communication/) now
retain completed working notes without streaming every intermediate note or
thinking update. Subagents, A2A calls, evaluations and health checks also avoid
updates their consumers do not use. Sent messages and task progress still arrive
when delivered.

Supported model drivers use native whole responses when the request permits it.
Subagent completion waits wake on lifecycle notifications and reconcile durable
state after a missed notification.

For live inspection, set this message control:

```rust
use everruns::{Controls, InputMessage};

let mut input = InputMessage::user("Check the release.");
input.controls = Some(Controls {
    hints: Some([("output_streaming".into(), true.into())].into()),
    ..Default::default()
});
```

Direct interactive chat continues streaming by default.
