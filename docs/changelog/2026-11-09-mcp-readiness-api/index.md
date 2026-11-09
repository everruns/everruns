---
title: Read MCP server readiness and reconnect from a Framework session
description: Framework hosts read per-server MCP status with failure reasons and probe times, reconnect a server on demand, and wait for readiness changes.
date: 2026-11-09
kind: feature
significance: notable
products: [framework]
surfaces: [code]
links:
  docs: [/framework/host-services/]
---

A Framework `Session` now reports the readiness of each configured MCP server.
A host can render MCP status, reconnect a server and print doctor output from
the session, without tracking that state itself. Enable the `mcp` crate
feature:

```rust
use everruns::Session;

async fn doctor(session: &Session) -> Result<(), everruns::SessionControlError> {
    let after = session.probe_mcp(Some("docs")).await?;
    for server in &after.servers {
        println!("{} {:?} {:?}", server.name, server.status, server.reason);
    }
    let mut changes = session.watch_mcp_readiness().await?;
    let _revision = changes.changed().await;
    Ok(())
}
```

Each entry includes the status, the failure or sign-in reason, the tool count,
the time of the last attempt and whether the server is deferred. A probe skips
the retry cooldown and the authentication hold. See
[MCP status, reconnect and doctor](/framework/host-services/#mcp-status-reconnect-and-doctor).
