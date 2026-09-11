---
title: Platform Management (removed)
description: Removed capability. Its management tools are superseded by the catalog-backed Platform capability.
---

> **This capability has been removed.** Use the
> [Platform capability](/capabilities/platform/) instead. Agents and harnesses
> that still reference `platform_management` keep running, but the capability
> contributes no tools and no system prompt.

| | |
|---|---|
| **ID** | `platform_management` |
| **Category** | Platform |
| **Status** | Retired |
| **Tools** | None |
| **Replacement** | [`platform`](/capabilities/platform/) |

## Why it was removed

Its tools were hand-written alongside the API rather than derived from it, so
they covered only harnesses, agents, apps, and sessions, and their schemas drifted
as the platform grew. The `platform` capability exposes the same surface through
`discover`, `query`, and `execute` over the server's registered command catalog,
which is the same inventory behind Everruns MCP, so it cannot drift.

## What to do

Replace the capability on any agent or harness that still lists it:

1. Open the agent or harness. A removed capability is flagged in its capability
   list.
2. Remove `platform_management` and add `platform`. Both are high-risk, so an
   admin performs the change.
3. Prompts that named the old tools should name the new flow instead: find the
   command with `discover`, read state with `query`, mutate with `execute`.

## Tool mapping

| Removed tool | Replacement |
|---|---|
| `read_capabilities` | `list_capabilities` / `get_capability` |
| `read_harnesses` | `list_harnesses` / `get_harness` |
| `manage_harnesses` | `create_harness`, `update_harness`, `delete_harness`, `copy_harness` |
| `read_agents` | `list_agents` / `get_agent` |
| `manage_agents` | `create_agent`, `update_agent`, `delete_agent` |
| `read_apps` | `list_apps` / `get_app` / `list_app_channels` |
| `manage_apps` | `create_app`, `update_app`, `delete_app`, `publish_app`, `unpublish_app` |
| `manage_app_channels` | `add_*_app_channel`, `update_app_channel`, `delete_app_channel` |
| `read_sessions` | `list_sessions` / `get_session` |
| `manage_sessions` | `create_session`, `delete_session` |
| `session_send_message` | `create_message` |
| `session_read_messages` | `list_messages` |
| `session_context_report` | `get_session_context_report` |
| `session_read_response` | No equivalent. It blocked until the turn finished; poll `list_messages` instead. |

Run `discover` for the exact current schema of any command in that table rather
than assuming the flags.

## Platform documentation

The embedded documentation mount at `/workspace/docs` moved to the
[Platform capability](/capabilities/platform/) unchanged.

## See also

- [Platform](/capabilities/platform/), the replacement
- [Capabilities Overview](/capabilities/)
