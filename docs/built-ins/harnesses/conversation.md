---
title: Conversation Harness
description: The default harness for simple dialogue, with no workspace and no shell.
---

Conversation inherits [Base](/built-ins/harnesses/base/) and adds the two chat affordances:
`ask_user` for structured questions and `message_metadata` so the model sees when each message was
sent. It provides no filesystem, bash, storage, web or delegation tools.

It is the organization default. Use it for chat assistants, support bots and Q&A. Dad Jokes uses
this harness plus `current_time`. Add task-specific capabilities on the agent when needed.

Workers do not inherit Conversation: [Worker](/built-ins/harnesses/worker/) builds on Base directly,
so an unattended worker does not pick up chat affordances it has no one to use with.

Choose `conversation` with the API's `harness_name` or the CLI's `--harness` option.

```rust
let harness = everruns::Harness::conversation();
```

Framework constructors load the same capability data as the hosted presets. Enable the host integrations required by the tools you use; a preset does not compile optional integrations into your application.

See [Harnesses](/features/harnesses/) for selection and inheritance.
