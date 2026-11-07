---
title: Start turns while slow MCP servers connect
description: Interactive hosts can limit waiting for cold MCP discovery, keep pending connections separate from failures, and use completed catalogs on later turns.
date: 2026-11-07
kind: improvement
significance: minor
products: [framework, platform, yolop]
surfaces: [agent, api, code, cli]
links:
  docs: [/framework/host-services/, /features/mcp/]
---

Framework hosts can set a short [MCP readiness budget](/framework/host-services/)
without cancelling slow discovery. Enable the `mcp` crate feature:

```rust
use std::time::Duration;
use everruns::hosting::services::HostServices;

let services = HostServices::new()
    .mcp_readiness_budget(Duration::from_millis(250));
```

Platform operators can set `EVERRUNS_MCP_DISCOVERY_READINESS_MS=250` for the
same [nonblocking MCP startup](/features/mcp/). Pending connections are distinct
from failed or authentication-required discovery. Complete catalogs become
available on later turns; partial catalogs are never exposed. Background work
retains bounded concurrency, memory and deadlines. Deferred servers still wait
for reveal, and the default remains blocking cold discovery.

## Update 2026-11-07

Yolop supports the same opt-in startup budget:

```sh
yolop --mcp-readiness-ms 250
```

Accepts `0..60000` milliseconds; omitted means blocking discovery. `/mcp` shows
pending discovery, ready, authentication required and discovery failed states.
The flag is unavailable with `--acp`.
Explicit context and tool-list queries still wait for full discovery. After
external OAuth login, run `/mcp reload` in the existing session.
