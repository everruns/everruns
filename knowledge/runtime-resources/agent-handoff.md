---
type: Specification
title: "Agent Handoff"
description: "Agent handoff behavior."
tags:
  - everruns
  - runtime-resources
---
# Agent Handoff

## Abstract

Agent handoff lets one agent delegate work to another configured first-party
Agent through an authorization and connection gate. Unlike blueprints, the
target is a normal Agent resource with its own prompt, capabilities, MCP
servers, mounted data, model defaults, and future identity bindings.

The source agent receives only handoff target access through `spawn_agent`. It does not receive the target
agent's tools and never receives provider credentials.

## Capability

Capability id: `agent_handoff`.

The capability is configured on the source agent with an allowlist of target
agents:

```json
{
  "targets": [
    {
      "id": "aws_operator",
      "name": "AWS Operator",
      "description": "Manage AWS infrastructure through fake AWS tools",
      "agent_id": "agent_01933b5a000070008000000000000001",
      "required_connections": ["fake_aws"],
      "required_scopes": ["fake_aws:rds:create"]
    }
  ]
}
```

Fields:

| Field | Description |
|---|---|
| `id` | Stable target key used in tool calls. |
| `name` | Human-readable target label and child-session title. |
| `description` | Prompt-visible summary for target selection. |
| `agent_id` | Public id of the configured target Agent. |
| `required_connections` | Provider ids that must be connected before handoff starts. |
| `required_scopes` | Non-secret audit labels recorded in resource metadata. |

`required_scopes` are labels in the first implementation. Provider-specific
tools that need hard authorization should enforce their own scoped grant checks
before performing external writes.

## Tools

### `spawn_agent` (`target.type = "agent"`)

Sessions with `agent_handoff` enabled advertise `agent` in the shared
`spawn_agent` dispatcher's `target.type` enum. The dispatcher can advertise
other active known delegation providers alongside `agent` (for example
subagents or external A2A), which is the migration path toward the single
delegation surface described in `knowledge/runtime-resources/session-tasks.md`.

Parameters:

| Field | Required | Description |
|---|---:|---|
| `name` | yes | Human-readable label for this delegated run and task. |
| `instructions` | yes | Work request for the target agent. Must not contain credentials. |
| `target.type` | yes | Must be `agent`. |
| `target.id` | yes | Target key from capability config. |
| `mode` | no | `background` by default when task tracking is available; `foreground` blocks for the child-session result; `invite` joins the target to the current session. |
| `public_context` | no | Non-secret structured context appended to the child task. |
| `result_schema` | no | JSON Schema for a required final machine result from the child session. |
| `message_schema` | no | JSON Schema for structured progress messages from the child session. |

Authorization, child-session creation, task kind, and result handling match the
original handoff contract for `background` and `foreground`. Background mode creates an
`agent_handoff` task with `wake_policy = on_terminal`, sends the initial
instructions from a detached watcher, heartbeats the task while waiting, and
settles the task with the child session's terminal status and last assistant
message. If `result_schema` is set, the child receives `report_result`; a valid
call writes `/.tasks/{task_id}/result.json`, and a successful child that omits
the call settles as `failed` with `error.kind = "no_result"`. Foreground mode
returns that JSON object instead of prose. If `message_schema` is set, the child
receives `report_task_progress`; valid data messages use `wake_policy =
on_activity` in background mode. These tools and validators are shared with
subagents. Invite mode rejects both schema options because it creates no child
task. If task tracking is unavailable, an omitted mode degrades to
`foreground`; explicit `background` is rejected.

Invite mode is for targets that can collaborate inside the host session's
environment. It adds the configured target Agent as a member participant in the
current session instead of creating a child session. Addressed user messages can
then route a turn to that participant. The target contributes only behavioral
agent overlay during addressed turns: prompt, capabilities, model defaults, and
client tools are folded on top of the host session environment. The target's
own harness stays reserved for child-session modes.

Behavior:

1. Validate the target exists in capability config.
2. Resolve required provider connections server-side through
   `UserConnectionResolver`.
3. If a connection is missing, return `connection_required(provider)`.
4. For `invite`, reject duplicate capability or mounted-resource conflicts with
   a clear error and add the target Agent as a member participant in the current
   session.
5. For `background` or `foreground`, create a child session with the target
   `agent_id`.
6. Persist parent/child metadata through the existing subagent session fields.
7. Create a `session_tasks` row of kind `agent_handoff` (distinct from
   `subagent`) with `links.child_session_id` set and `spec` carrying
   `target_id`/`external_agent_id`/`mode` and any declared result or message
   schema.
8. Send the task to the child session.
9. In foreground mode, block until the child session idles and return its
   validated structured result when declared, otherwise its last assistant
   message.

### Monitoring and Steering

Use the generic task tools after spawning. `list_tasks(kind="agent_handoff")`
lists handoffs for the current session. `get_task` reads handoff state and
result, `message_task` sends follow-up input to the child session,
`cancel_task` requests cooperative cancellation, and `wait_task` waits for a
terminal or interrupted state. Because handoffs have their own kind,
`list_tasks(kind="subagent")` and `list_tasks(kind="agent_handoff")` are
disjoint.

## Security Contract

Credentials must never be passed through handoff tool arguments, target config,
messages, events, resource metadata, or prompt context. Required provider
connections are checked inside the server process. Tools that need credentials
resolve them lazily during execution via `UserConnectionResolver`.

The source agent gets authority to request a handoff to configured targets. It
does not get authority to call the target agent's tools directly.

The first implementation is a configured-agent delegation gate. It does not yet
create a separate durable scoped grant row. Provider tools that need
fine-grained write protection should add scoped grant checks before production
use against real external infrastructure.

## Fake AWS Example

Use this setup to test the flow locally. The `fake_aws` capability and its
connector are demo fixtures in `everruns-test-support` (EVE-875); they are not
registered by product binaries, so this walkthrough requires a local build
that registers them explicitly (integration tests do this, see
`crates/server/src/grpc_service/tests.rs` for the automated equivalent).

1. Create an `AWS Operator` agent with `fake_aws` enabled.
2. Create a `Welcome` agent with `agent_handoff` enabled and config:

```json
{
  "targets": [
    {
      "id": "aws_operator",
      "name": "AWS Operator",
      "agent_id": "<AWS_OPERATOR_AGENT_ID>",
      "required_connections": ["fake_aws"],
      "required_scopes": ["fake_aws:rds:create"]
    }
  ]
}
```

3. Start a session with the `Welcome` agent and ask:

```text
Create an RDS database named app-db in us-east-1.
```

4. The first handoff should return `connection_required: "fake_aws"` if the
   fake AWS connection is not configured.
5. Add a Fake AWS connection in Settings → Connections. Any non-empty key is
   accepted by the fake provider for local testing.
6. Retry the user request. The welcome agent should call
   `spawn_agent` with `target.type = "agent"`, and the child AWS Operator session
   should receive the task and use its own fake AWS tools.

Background and foreground modes are both available through `spawn_agent`.

Focused verification:

```bash
cargo test -p everruns-core agent_handoff -- --nocapture
```
