---
title: Sandbox Fleet
description: Create and operate multiple explicitly addressed resource Sandboxes without changing a session's primary Sandbox.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `sandbox_fleet` |
| **Category** | Sandboxes |
| **Tools** | 8 |
| **Dependency** | [`session_storage`](/capabilities/session-storage/) |

Sandbox Fleet gives an Agent provider-neutral tools for parallel or isolated compute. Daytona backs
the capability today, but the model sees only Everruns logical Sandbox IDs. Provider resource IDs,
credentials, and translation stay in the trusted control plane.

These are **resource Sandboxes**. They never replace the session's primary Sandbox: ordinary shell
and file tools continue to use the primary Sandbox configured by the Harness and Agent.

| Tool | Purpose |
|---|---|
| `sandbox_create` | Create a resource Sandbox and return its session-scoped logical ID |
| `sandbox_exec` | Run a command in an explicitly named Sandbox |
| `sandbox_read_file` | Read a file from an explicitly named Sandbox |
| `sandbox_write_file` | Write a file in an explicitly named Sandbox |
| `sandbox_list` | List the current session's resource Sandboxes |
| `sandbox_inspect` | Inspect one resource Sandbox without exposing provider bindings |
| `sandbox_checkpoint` | Copy its workspace into durable session file storage |
| `sandbox_manage` | Stop or delete it |

Every operation after creation requires `sandbox_id`. An ID from another session is rejected. Delete
resource Sandboxes when the work is complete.

The older [`daytona`](/capabilities/daytona/) capability remains available for existing Agents and
provider-specific operations. New provider-neutral fleet workflows should use `sandbox_fleet`.
