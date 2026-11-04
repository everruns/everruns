---
title: "Run commands and hot-install capabilities on live sessions"
description: "Framework sessions run slash commands, activate host-registered capabilities and switch model, effort and MCP servers without a runtime handle."
date: 2026-10-19
kind: feature
significance: notable
products: [framework]
release: "0.49.0"
surfaces: [code]
links:
  docs: [/framework/session-controls/]
---

A host driving a Framework `Session` can now discover and run capability slash
commands, register an extension on its `Engine` and activate or deactivate it
per session, and change model, reasoning effort and MCP servers while the
session is live. Commands work from a tool of the running turn, and changes
apply from the next model call.

```rust
use everruns::{CapabilityRef, ReasoningEffort, Session};

async fn tune(session: &Session) -> Result<(), everruns::SessionControlError> {
    session.activate_capability(CapabilityRef::new("btw")).await?;
    session.run_command("/btw", Some("which files changed?")).await?;
    session.set_reasoning_effort(Some(ReasoningEffort::High)).await?;
    Ok(())
}
```

Unavailable selections fail without falling back or changing saved state. See
[Session Controls](/framework/session-controls/).
