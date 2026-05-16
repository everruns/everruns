---
title: Sub Agents
description: Spawn and manage subagents for parallel task execution in isolated context windows. Orchestrate multi-agent workflows with message passing and lifecycle control.
---

| | |
|---|---|
| **ID** | `subagents` |
| **Category** | Orchestration |
| **Features** | `subagents` |
| **Dependencies** | None |

Spawn and manage subagents for parallel task execution. Each subagent runs in its own isolated context window, allowing the parent agent to delegate verbose or independent tasks without cluttering the main conversation. Subagents inherit the parent's harness and agent configuration but operate with their own message history.

## Tools

### `spawn_subagent`

Create and start a new subagent. The subagent begins executing immediately in the background unless foreground mode is used.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Human-readable name for the subagent. Must be unique within the session. |
| `task` | string | yes | Description of what the subagent should do. This becomes the subagent's initial prompt. |

### `get_subagents`

List subagents or retrieve details for a specific one, including status and output.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `name_or_id` | string | no | Specific subagent by name or session ID. Omit to list all subagents. |
| `status_filter` | string | no | Filter by status: `all`, `running`, `completed`, `failed`. Defaults to `all`. |

### `message_subagent`

Send a follow-up message to an existing subagent. Use this to steer a running subagent, ask clarifying questions, or gracefully stop it.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `name_or_id` | string | yes | Subagent name or session ID. |
| `message` | string | yes | Message to send to the subagent. |
| `cancel` | boolean | no | Gracefully stop the subagent after delivering the message. |

## Notes

- **No nesting** — subagents cannot spawn other subagents.
- **Case-insensitive matching** — names are matched case-insensitively when using `name_or_id`.
- **Foreground mode** — spawning can block until the subagent completes. Foreground execution has a 5-minute timeout.
- **Inherited configuration** — subagents inherit the parent's harness and agent configuration.

## See Also

- [Session](./session.md) — session metadata and lifecycle
- [Platform Management](./platform-management.md) — agent and platform configuration
- [Capabilities Overview](./index.md) — full list of available capabilities
