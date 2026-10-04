---
title: Conversation Harness
description: The default harness for simple dialogue.
---

Conversation inherits Base and adds context compaction, standard error disclosure, tool-call repair and loop detection. It provides no filesystem, bash, storage, web or delegation tools.

Dad Jokes uses this harness plus `current_time`. Add task-specific capabilities on the agent when needed.

Choose `conversation` with the API's `harness_name` or the CLI's `--harness` option.

```rust
let harness = everruns::Harness::conversation();
```

Framework constructors load the same capability data as the hosted presets. Enable the host integrations required by the tools you use; a preset does not compile optional integrations into your application.

See [Harnesses](/features/harnesses/) for selection and inheritance.
