---
title: Worker Harness
description: A worker with skills, long context and task coordination.
---

Worker inherits Worker Base and adds skills, history retrieval, automatic tool discovery, budget awareness, todo lists, subagents and task controls.

Subagents create separate child sessions. Ordinary children inherit the parent configuration; blueprints provide specialist behavior. Depth and root-tree task limits bound delegation. Assigning this harness requires permission to assign its high-risk capabilities.

Web access, secret/KV tools, organization memory mounts and session scheduling remain opt-in. Recurring agent execution is configured with Agent Triggers.

Choose `worker` with the API's `harness_name` or the CLI's `--harness` option.

```rust
let harness = everruns::Harness::worker();
```

Framework constructors load the same capability data as the hosted presets. Enable the host integrations required by the tools you use; a preset does not compile optional integrations into your application. Delegation also needs a host with subagent and task backends. The default in-memory framework host does not supply these services; unregistered capability references remain inert.

See [Harnesses](/features/harnesses/) for selection and inheritance.
