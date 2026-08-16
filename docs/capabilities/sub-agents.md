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

### `spawn_agent`

Sessions with `subagents` expose `target.type: "subagent"` in the shared
`spawn_agent` dispatcher. If first-party handoffs or external A2A delegation are
also active, the same tool advertises those target types too. The dispatcher
returns a `task_id` for the generic session task tools and moves Everruns toward
one delegation surface across subagents, first-party agent handoffs, and
external A2A agents.

Create and start a new subagent by calling `spawn_agent` with
`target.type: "subagent"`. By default the subagent runs in the background: the
tool returns immediately with a `task_id`, the parent agent keeps working, and
the session is notified when the subagent finishes. Use the `task_id` with the
generic session task tools to monitor, message, or cancel the subagent.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Human-readable name for the subagent. Must be unique within the session. |
| `instructions` | string | yes | Instructions for what the subagent should do. This becomes the subagent's initial prompt. |
| `target.type` | string | yes | Must be `subagent`. |
| `mode` | string | no | `background` (default) returns immediately with a `task_id`; `foreground` blocks until the subagent completes and returns its result inline. |
| `blueprint` | string | no | Optional specialist blueprint ID, such as `github_scout`, that supplies its own prompt, model, and private tools. |
| `config` | object | no | Blueprint-specific configuration. Only valid when `blueprint` is set. |

## Managing subagents after spawn

Use the generic `session_tasks` tools to monitor and steer subagents after spawning. The `task_id` is returned by `spawn_agent`.

- `list_tasks` with `kind: "subagent"`, list all subagent tasks and their status
- `get_task` with the task ID, get detailed status and result for a specific subagent
- `message_task`, send a steering message or additional context to a running subagent
- `cancel_task`, request cooperative cancellation of a subagent
- `wait_task`, block until a subagent reaches a terminal or interrupted state

## Notes

- **Governed spawning**: subagents can spawn nested subagents up to `max_subagent_depth` (default 2); set it to 0 to block subagent spawning. Each root session also has `max_active_descendant_tasks` (default 16) and `max_total_descendant_tasks` (default 200) caps to bound wide fan-out and repeated spawn loops.
- **Shared budget pool**: nested subagents spend from the root session's session-scoped budget.
- **Background mode (default)**: spawning returns immediately with a `task_id`. The final result lands on the task record (`summary` via `get_task`), and the parent session is woken when the subagent reaches a terminal state. Background runs are capped at 6 hours.
- **Foreground mode**: `mode: "foreground"` blocks until the subagent completes and returns its result inline. Foreground execution has a 5-minute timeout.
- **Inherited configuration**: subagents inherit the parent's harness and agent configuration.
- **Blueprints**: specialist blueprints can run with their own prompt, model, and private tools while still using the same subagent lifecycle.

## See Also

- [`knowledge/runtime-resources/session-tasks.md`](https://github.com/everruns/everruns/blob/main/knowledge/runtime-resources/session-tasks.md), generic task monitoring and control (`list_tasks`, `get_task`, `message_task`, `cancel_task`, `wait_task`)
- [GitHub Scout](/capabilities/github-scout/), blueprint-only GitHub repository exploration
- [Session](/capabilities/session/), session metadata and lifecycle
- [Platform](/capabilities/platform/), agent and platform configuration
- [Capabilities Overview](/capabilities/), full list of available capabilities
