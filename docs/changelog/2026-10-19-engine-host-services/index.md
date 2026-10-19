---
title: "Bring host capabilities and providers into Engine sessions"
description: "Embedding hosts attach their own capabilities, provider resolution, retry budgets, MCP credentials, request attribution and observers to Engine sessions."
date: 2026-10-19
kind: feature
significance: notable
products: [framework]
surfaces: [code]
links:
  docs: [/framework/host-services/]
---

An application that already owns guarded capabilities, credential resolution
or event observers can now keep them when `Engine` runs its sessions.
`everruns::hosting::services::HostServices` carries Core capabilities with
pre-tool hooks, model views and slash commands, host provider resolution,
retry and stream-stall budgets, call-time MCP credentials, and
credential-free request metadata such as the app version and OpenRouter
attribution that every provider request carries.

```rust
use everruns::hosting::services::HostServices;
use everruns::{Agent, BuildError, Model};

fn agent(services: HostServices) -> Result<Agent, BuildError> {
    Agent::builder()
        .instructions("You maintain this repository.")
        .model(Model::on("team-openai", "gpt-6.1-sol"))
        .host_services(services)
        .build()
}
```

Collisions with Framework capabilities, conflicting provider declarations and
request metadata that claims a correlation id such as `session_id` fail the
build, and the session filesystem stays the one the Environment chose.
See [Host Services](/framework/host-services/).
