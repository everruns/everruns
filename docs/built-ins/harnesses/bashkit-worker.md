---
title: Bashkit Worker Harness
description: Worker plus the Bashkit virtual shell, sealed to one recoverable Bashkit virtual workspace.
---

**Bashkit Worker** extends [Worker](/built-ins/harnesses/worker/) with the Bashkit shell and fixes
the primary Sandbox to Everruns' managed Bashkit Virtual Workspace.

| | |
|---|---|
| **Name** | `bashkit-worker` |
| **Parent** | `worker` |
| **Adds** | `bashkit_shell` |
| **Sandbox policy** | Fixed; Agent and Session overrides are rejected |
| **Primary Sandbox** | Managed Bashkit Virtual Workspace |

Use Bashkit Worker for support, operations, and general assistants that need a durable virtual
filesystem and Bash-like commands but do not need native packages, processes, ports, or PTYs. It
needs no sandbox provider, and the logical Sandbox and workspace survive replacement of runtime
compute. [Platform Chat](/built-ins/harnesses/platform-chat/) runs on it.

Choose [Sandbox Worker](/built-ins/harnesses/sandbox-worker/) when an agent needs a real machine, or
[Worker](/built-ins/harnesses/worker/) when it needs no shell. Add `sandbox_fleet` separately when an
Agent should keep its primary workspace while creating and operating multiple explicitly addressed
resource Sandboxes.

Custom Harnesses may inherit from Bashkit Worker, but they inherit the sealed Sandbox policy as
well. Renaming the child or changing its capabilities does not make the primary Sandbox overridable.

```rust
let harness = everruns::Harness::bashkit_worker();
```
