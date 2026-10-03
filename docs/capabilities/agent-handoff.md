---
title: Agent Handoff
description: Delegate work to configured first-party agents with spawn_agent, as a background task, a blocking call, or an invited participant in the current session.
appliesTo: [platform]
---

| | |
|---|---|
| **ID** | `agent_handoff` |
| **Category** | Orchestration |
| **Features** | `agent_handoffs` |
| **Dependencies** | None |
| **Risk** | High |
| **Availability** | Experimental, dev-only |

Agent Handoff lets an agent hand work to other first-party Everruns agents.
Each target is configured ahead of time with an agent and a harness, so the
model can only pick from that list. The capability adds the `agent` target type
to the shared `spawn_agent` tool, the same dispatcher that
[Sub Agents](/capabilities/sub-agents/) and
[A2A Agent Delegation](/capabilities/a2a-agent-delegation/) extend. The child runs as its own session, or joins
the current session as a member.

**Experimental.** The capability is registered when
`FEATURE_AGENT_DELEGATION` is on. That flag is on by default at the development
deployment grade and off in production, so Agent Handoff is not available on
Everruns Cloud.

For delegation to agents outside Everruns, see
[A2A Agent Delegation](/capabilities/a2a-agent-delegation/). For ad hoc child
sessions that use the parent's own configuration, see
[Sub Agents](/capabilities/sub-agents/).

## Configuration

```json
{
  "ref": "agent_handoff",
  "config": {
    "targets": [
      {
        "id": "billing",
        "name": "Billing agent",
        "description": "Answers invoice and refund questions",
        "agent_id": "agent_...",
        "harness_id": "harness_...",
        "required_connections": ["github"]
      }
    ]
  }
}
```

| Field | Required | Description |
|---|---|---|
| `id` | Yes | Stable key the model passes as `target.id` |
| `name` | Yes | Human-readable name, shown in the system prompt |
| `description` | No | What the target does, shown in the system prompt |
| `agent_id` | Yes | Public ID of the target agent |
| `harness_id` | Yes | Public ID of the harness the target runs on |
| `required_connections` | No | Provider connections that must exist before the handoff starts |
| `required_scopes` | No | Non-secret scope labels recorded for audit and resource metadata |

The system prompt lists every configured target by name, ID, and description.

## `spawn_agent`

| Parameter | Required | Description |
|---|---|---|
| `name` | Yes | Name for this delegated run |
| `instructions` | Yes | Instructions for the target. Must not contain credentials |
| `target` | Yes | `{ "type": "agent", "id": "<target id>" }` |
| `mode` | No | `background`, `foreground`, or `invite` |
| `public_context` | No | Non-secret structured context, appended to the instructions |
| `result_schema` | No | JSON Schema for the child's final result. The child must call `report_result` before the task can succeed |
| `message_schema` | No | JSON Schema for structured progress messages. The child gets `report_task_progress` |

### Modes

| Mode | Behavior |
|---|---|
| `background` | Default when session task tracking is available. Returns a `task_id` right away |
| `foreground` | Default otherwise. Blocks until the handoff finishes and returns the result |
| `invite` | Adds the target agent as a member of the current session instead of starting a new one |

A background handoff is tracked as a session task of kind `agent_handoff`, so
the parent manages it with the session task tools (`list_tasks`, `get_task`,
`message_task`, `cancel_task`, `wait_task`). Background mode fails when task
tracking is not available.

Invite mode does not create a child task, so it rejects `result_schema` and
`message_schema`. It also refuses the invite when the target's configuration
conflicts with the current session's: the same capability with a different
config, the same mounted file path with different contents, or the same MCP
server name with a different definition.

### Connections

When a target lists `required_connections` and the session cannot resolve a
connection for one of those providers, `spawn_agent` returns a
`connection_required` result naming the provider. The client collects the
credentials through the Connections flow. The system prompt tells the agent
never to ask for tokens in chat or pass them in tool arguments.

## See also

- [A2A Agent Delegation](/capabilities/a2a-agent-delegation/), delegation to external agents
- [Sub Agents](/capabilities/sub-agents/), child sessions from the parent's own configuration, and the session task tools
