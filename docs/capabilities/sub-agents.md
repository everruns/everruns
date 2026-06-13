---
title: Sub Agents
description: Spawn subagents for parallel task execution in isolated context windows. Orchestrate multi-agent workflows with the generic session task tools.
---

| | |
|---|---|
| **ID** | `subagents` |
| **Category** | Core |
| **Features** | `subagents` |
| **Dependencies** | None |

Spawn subagents for parallel task execution. Each subagent runs in its own isolated context window, allowing the parent agent to delegate verbose or independent tasks without cluttering the main conversation. Subagents inherit the parent's harness and agent configuration but operate with their own message history.

## Tools

### `spawn_subagent`

Create and start a new subagent. The subagent begins executing immediately. Returns a `task_id` that can be used with the generic session task tools to monitor, message, or cancel the subagent.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Human-readable name for the subagent. Must be unique within the session. |
| `instructions` | string | yes | Instructions for what the subagent should do. This becomes the subagent's initial prompt. |
| `blueprint` | string | no | Optional specialist blueprint ID, such as `github_scout`, that supplies its own prompt, model, and private tools. |
| `config` | object | no | Blueprint-specific configuration. Only valid when `blueprint` is set. |

## Managing subagents after spawn

Use the generic `session_tasks` tools to monitor and steer subagents after spawning. The `task_id` is returned by `spawn_subagent`.

- `list_tasks` with `kind: "subagent"` — list all subagent tasks and their status
- `get_task` with the task ID — get detailed status and result for a specific subagent
- `message_task` — send a steering message or additional context to a running subagent
- `cancel_task` — request cooperative cancellation of a subagent
- `wait_task` — block until a subagent reaches a terminal or interrupted state

## Notes

- **No nesting** — subagents cannot spawn other subagents.
- **Foreground mode** — spawning blocks until the subagent completes. Foreground execution has a 5-minute timeout.
- **Inherited configuration** — subagents inherit the parent's harness and agent configuration.
- **Blueprints** — specialist blueprints can run with their own prompt, model, and private tools while still using the same subagent lifecycle.

## See Also

- [`specs/session-tasks.md`](https://github.com/everruns/everruns/blob/main/specs/session-tasks.md) — generic task monitoring and control (`list_tasks`, `get_task`, `message_task`, `cancel_task`, `wait_task`)
- [GitHub Scout](/capabilities/github-scout/) — blueprint-only GitHub repository exploration
- [Session](/capabilities/session/) — session metadata and lifecycle
- [Platform Management](/capabilities/platform-management/) — agent and platform configuration
- [Capabilities Overview](/capabilities/) — full list of available capabilities
