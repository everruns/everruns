---
title: Sub Agents
description: Spawn and manage subagents for parallel task execution in isolated context windows.
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

## Use Cases

- **Delegate verbose tasks** (test runs, large searches) to keep the main conversation clean and focused.
- **Run independent tasks in parallel** — e.g., run tests and review code at the same time.
- **Steer or redirect a running subagent** with new instructions via `message_subagent`.
- **Resume a completed subagent** with follow-up questions to drill deeper into results.

## Example

A user asks the agent to analyze a codebase for quality issues:

```
User: Review the codebase — run the test suite and check for code style issues.

Agent: I'll delegate these to two subagents running in parallel.

→ spawn_subagent(name: "Test Runner", task: "Run the full test suite with `cargo test --all-features`. Report any failures with file paths and error messages.")

→ spawn_subagent(name: "Code Reviewer", task: "Check all Rust source files for clippy warnings, formatting issues, and common anti-patterns. Summarize findings by crate.")

Agent: Both subagents are running. Let me check on their progress.

→ get_subagents(status_filter: "running")

[After some time...]

→ get_subagents(name_or_id: "Test Runner")

Agent: Tests finished with 2 failures. Let me get more detail on the code review.

→ message_subagent(name_or_id: "Code Reviewer", message: "Focus on the crates/core and crates/api crates specifically. Any unsafe code or missing error handling?")
```

## Notes

- **No nesting** — subagents cannot spawn other subagents.
- **Case-insensitive matching** — names are matched case-insensitively when using `name_or_id`.
- **Foreground mode** — spawning can block until the subagent completes. Foreground execution has a 5-minute timeout.
- **Inherited configuration** — subagents inherit the parent's harness and agent configuration.

## See Also

- [Session](./session.md) — session metadata and lifecycle
- [Platform Management](./platform-management.md) — agent and platform configuration
- [Capabilities Overview](./index.md) — full list of available capabilities
