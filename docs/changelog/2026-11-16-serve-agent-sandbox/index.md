---
title: Declare Serve sandboxes alongside each agent
description: Select Bashkit and workspace permissions in the Serve agent builder, with per-agent manifest declarations and existing host storage bindings preserved.
date: 2026-11-16
kind: improvement
significance: minor
products: [framework]
surfaces: [code]
links:
  docs: [/framework/serve/]
---

[Serve](/framework/serve/) agents can declare their sandbox beside their model and workspace permissions in `agent.rs`. Different agents in one app can select different sandboxes.

```rust
use serve::prelude::*;

let agent = Agent::builder()
    .model("openai/gpt-6-luna")
    .sandbox(SandboxKind::Bashkit)
    .workspace_policy(everruns::WorkspacePolicy::read_only())
    .build();
```

Selecting Bashkit does not grant writes. Use a scoped workspace policy for writable paths. Explicit agent permissions cap host adapter grants while preserving host restrictions. Hosts still supply storage and MicroVM adapters. Existing `serve.toml` sandbox settings remain a fallback for agents without an explicit selection; manifests now report each agent's effective sandbox.
