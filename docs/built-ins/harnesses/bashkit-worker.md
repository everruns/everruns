---
title: Bashkit Worker Harness
description: General-purpose Worker behavior with one sealed, recoverable Bashkit virtual workspace.
---

**Bashkit Worker** extends [Worker](/built-ins/harnesses/worker/) and fixes the primary Sandbox to
Everruns' managed Bashkit Virtual Workspace.

| | |
|---|---|
| **Name** | `bashkit-worker` |
| **Parent** | `worker` |
| **Environment policy** | Fixed; Agent and session overrides are rejected |
| **Primary Sandbox** | Managed Bashkit Virtual Workspace |

Use Bashkit Worker for support, operations, and general assistants that need a durable virtual
filesystem and Bash-like commands but do not need native packages, processes, ports, or PTYs. The
logical Sandbox and workspace survive replacement of runtime compute.

Choose provider-neutral [Worker](/built-ins/harnesses/worker/) or
[Worker Base](/built-ins/harnesses/worker-base/) when an Agent must bind a different reusable
Environment. Add `sandbox_fleet` separately when an Agent should keep its primary workspace while
creating and operating multiple explicitly addressed resource Sandboxes.

Custom Harnesses may inherit from Bashkit Worker, but they inherit the sealed Environment policy as
well. Renaming the child or changing its capabilities does not make the primary Sandbox overridable.
