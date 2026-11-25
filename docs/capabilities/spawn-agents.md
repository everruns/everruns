---
title: Spawn agents
description: One-off calls to other agents with spawn_agent. A brief goes in, a result comes back, and the spawned agent is done.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `spawn_agents` |
| **Category** | Orchestration |
| **Features** | `spawn_agents` |
| **Dependencies** | Session Tasks |

Spawn agents lets an agent make **one-off calls** to agents: it sends a brief,
the spawned agent works it in its own session and context, and its result
comes back. Nothing is followed afterwards. Use it for a review, a lookup, a
check, or research that runs in parallel.

An agent starts other agents one of two ways, and never both:

| | Spawn agents | [Threads](/capabilities/threads/) |
|---|---|---|
| Shape | A call: brief in, result out | Followed work the agent messages and closes |
| The person | Talks only to the agent | Can come back to a thread, steer it, accept it |
| Tools | `spawn_agent` + the task tools | `start_thread`, `message_thread`, `list_threads`, `get_thread`, `close_thread` |

Threads already cover one-off work, so saving both on one agent is refused.

## Configuration

```json
{
  "ref": "spawn_agents",
  "config": {
    "self": true,
    "agents": [
      { "agent_id": "agent_...", "required_connections": ["github"] }
    ],
    "max_open": 4
  }
}
```

| Field | Default | Description |
|---|---|---|
| `self` | `true` | The agent may spawn a fresh copy of itself, with its own context |
| `agents` | none | Other agents it may spawn, each as itself with its own tools and files |
| `any_agent` | `false` | Any agent the person may run, by agent id |
| `max_depth` | 2 | How deep spawned agents may nest; 0 stops nesting |
| `max_open` | 16 | Spawned agents working at once, across the whole run |
| `max_total` | 200 | Agents one run may spawn in total |
| `helper_models`, `helper_model_pick` | `same` only | Models a copy of itself may run on, see [Helper models](#helper-models) |

An entry in `agents`:

| Field | Required | Description |
|---|---|---|
| `agent_id` | Yes | Public ID of the agent |
| `name`, `description` | No | What the model sees; filled from the agent when saved |
| `files` | No | `copy` starts each call with a copy of the caller's files |
| `required_connections` | No | Provider connections that must exist before the call starts |

In the UI, add agents from the agent's **Team** tab.

## `spawn_agent`

| Parameter | Required | Description |
|---|---|---|
| `name` | Yes | A short name for this call |
| `instructions` | Yes | The complete brief. The spawned agent sees nothing else |
| `agent` | No | `self` (default) or an agent's name from the list |
| `wait` | No | `true` blocks until the result is back. Default: the call runs in the background and returns a `task_id` |
| `goal` | No | What done looks like, shown on the task |
| `model` | No | A helper model key (copies of itself only) |
| `blueprint`, `config`, `sandbox` | No | A specialist blueprint, its config, and a sandbox (copies of itself only) |
| `result_schema` | No | JSON Schema for the result; the spawned agent must call `finish` with it |
| `message_schema` | No | JSON Schema for progress messages (`report_progress`) |
| `public_context` | No | Non-secret structured context appended to the brief |

The spawned agent gets `finish` to return its result and `report_progress`
for progress, from the host. A background call is a session task, so the
caller follows it with `get_task`, `wait_task`, `message_task`,
`cancel_task` and `list_tasks`, and is woken when it finishes.

With agents in the list, the agent also gets `invite_agent`, which adds one of
them to the current conversation as a participant instead of starting a
separate session.

## Helper models

An agent lists the models a copy of itself may run on as **helper models**:
keys the model names, each with a "when to use" line. The model never sees a
raw model id.

| Key | Runs on | Typical use |
|---|---|---|
| `same` | The parent's model (the default) | Most calls |
| `fast` | The Small job | Search, lookups, summaries |
| `coding` | The Coding job | Implementation work |
| `deep` | The Research job | Hard reasoning, planning |
| `second_opinion` | The Default job of a provider other than the parent's | Verifier, reviewer |

Edit the list on the agent page under **Helper models**, and choose who picks:
the agent, or **Auto**, where the organization's Decision model picks. The
list lives in this capability's config:

```json
{"ref": "spawn_agents", "config": {
  "self": true,
  "helper_models": [
    {"key": "same", "when": "Most work.", "target": {"type": "same"}},
    {"key": "verifier", "when": "Review a finished change.",
     "target": {"type": "other_provider", "job": "default"}}
  ],
  "helper_model_pick": "agent"
}}
```

## Shared folder

Every agent in one run shares a `shared/` folder in its files. What one agent
writes there, the others read straight away. Pass file paths in the brief
instead of pasting long content, and ask agents to write results there.

## Connections, approvals and questions

When a listed agent needs a connection the session does not have,
`spawn_agent` returns `connection_required` naming the provider, and the
person connects it in Connections. When a spawned agent needs an approval or
asks a question, the person answers it in the conversation they started; Chat
shows the card headed with that agent's name. Another agent's message never
approves.

## Notes

- Spawned agents spend from the root session's budget.
- Background calls are capped at 6 hours; `wait` calls at 5 minutes.
- A copy of itself inherits the agent's harness and configuration.
- Agents outside Everruns: [A2A Agent Delegation](/capabilities/a2a-agent-delegation/)
  adds them to the same `spawn_agent` tool.
- The retired ids `subagents` and `agent_handoff` are rewritten into
  `spawn_agents` when an agent is read or saved.

## See also

- [Threads](/capabilities/threads/), followed work
- [Sibling communication](/capabilities/sibling-communication/), spawned agents messaging each other
- [Author an agent blueprint](/framework/agent-blueprints/)
- [Framework: agents that start agents](/framework/coordination/)
