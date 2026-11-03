---
title: Agents
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
| **Availability** | Experimental |

The Agents capability (`agent_handoff`) lets an agent hand work to other
first-party Everruns agents. The agents it can use are listed ahead of time,
so the model can only pick from that list. In the UI, add them from the
agent's **Agents** tab: link an existing agent, or create a new one there.
Agents created there are exported and imported together with the agent that
uses them. The capability adds the `agent` target type
to the shared `spawn_agent` tool, the same dispatcher that
[Sub Agents](/capabilities/sub-agents/) and
[A2A Agent Delegation](/capabilities/a2a-agent-delegation/) extend. The child runs as its own session, or joins
the current session as a member.

**Experimental.** Adding the capability to an agent is the opt-in; there is no
separate feature flag.

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
        "agent_id": "agent_...",
        "required_connections": ["github"]
      }
    ]
  }
}
```

| Field | Required | Description |
|---|---|---|
| `agent_id` | Yes | Public ID of the target agent |
| `id` | No | Key the model passes as `target.id`; filled from the agent when saved |
| `name` | No | Shown in the system prompt; filled from the agent's name when saved |
| `description` | No | Shown in the system prompt; filled from the agent's description when saved |
| `harness_id` | No | Harness to run the target on; defaults to the agent's current harness at hand-off time |
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

### Approvals and questions

When an agent the session handed work to needs an approval or asks a
question, the person answers it in the conversation they started, not in the
other agent's session. Chat and Playground show the card headed with that
agent's name.

Over the API, `GET /v1/sessions/{session_id}/run-requests` on the first
session lists what the other agents wait on, each with the asking agent and
its `session_id`. Answer with the usual
`POST /v1/sessions/{session_id}/tool-approvals` or `…/question-answers` on
either session: the first session routes the answer to the agent that asked.
On the Agent Execution API, `pending_approvals` and `pending_questions` on the
session include these requests with `agent` and `session_id`, and answers on
the session are routed the same way. Only a person (or the API key holder)
answers; another agent's message never approves.

## See also

- [A2A Agent Delegation](/capabilities/a2a-agent-delegation/), delegation to external agents
- [Sub Agents](/capabilities/sub-agents/), child sessions from the parent's own configuration, and the session task tools
