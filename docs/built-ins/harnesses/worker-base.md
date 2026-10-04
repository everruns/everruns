---
title: Worker Base Harness
description: Files, bash and project instructions for specialized workers.
---

Worker Base inherits Conversation and adds working files, bash, project instructions, session tools, parallel tool calls, output persistence/distillation and approval guidance. It does not include skills or subagents.

Select the execution target through the agent Environment. Bashkit is the default shell; another selected environment replaces that compute attachment.

Hosted filesystem access includes the existing server-managed agent and owner memory paths. Arbitrary organization memory mounts remain explicit. Web fetching, secret/KV tools, session scheduling and citations are opt-in.

Choose `worker-base` with the API's `harness_name` or the CLI's `--harness` option.

```rust
let harness = everruns::Harness::worker_base();
```

Framework constructors load the same capability data as the hosted presets. Enable the host integrations required by the tools you use; a preset does not compile optional integrations into your application.

See [Harnesses](/features/harnesses/) for selection and inheritance.
