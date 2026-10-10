---
title: A2A Agent Delegation
description: Delegate work to configured external agents over the A2A protocol through spawn_agent, in the foreground or as a background session task.
appliesTo: [framework, platform]
---

| | |
|---|---|
| **ID** | `a2a_agent_delegation` |
| **Category** | Orchestration |
| **Features** | `agent_runs` |
| **Dependencies** | `session_tasks` |
| **Risk** | High |
| **Availability** | Experimental |

A2A Agent Delegation lets an agent hand work to agents that run outside
Everruns and speak the [A2A protocol](/features/a2a/). The agents it may call
are listed in the capability's configuration, so the model can only reach URLs
you configured. The capability adds the `external_a2a` target type to the
shared `spawn_agent` tool, the same dispatcher that
[Sub Agents](/capabilities/sub-agents/) and
[Agent Handoff](/capabilities/agent-handoff/) extend.

**Experimental.** On the platform, adding the capability to an agent is the
opt-in; there is no separate organisation feature flag. An operator can set
`FEATURE_AGENT_DELEGATION=off` to leave delegation out of a deployment entirely.
In the [Framework](/framework/), turn on the `a2a` Cargo feature of
`everruns` (it implies `local`) and add the capability by ID. See
[A2A](/features/a2a/#a2a-in-the-framework).

## Configuration

```json
{
  "ref": "a2a_agent_delegation",
  "config": {
    "agents": [
      {
        "id": "research",
        "name": "Research agent",
        "description": "Researches a topic and returns notes.",
        "base_url": "https://agents.example.com/research"
      }
    ]
  }
}
```

| Field | Required | Description |
|---|---|---|
| `id` | Yes | Stable key the model passes as `target.id` |
| `name` | Yes | Human-readable name, shown in the system prompt |
| `description` | No | What the agent does, shown in the system prompt |
| `base_url` | One of `base_url` or `agent_card` | Where the Agent Card is fetched from (`/.well-known/agent-card.json`) |
| `agent_card` | One of `base_url` or `agent_card` | An inline Agent Card, used instead of discovery |
| `headers` | No | Static headers sent to the A2A endpoint. They are stored in the config, so they must not hold secrets |
| `preferred_binding` | No | `JSONRPC` or `HTTP+JSON` |
| `poll_interval_ms` | No | How often to poll the remote task, 100 to 60000. Default 1000 |
| `allow_local_urls` | No | Allows localhost and private addresses for local development. Honored only when `DEPLOYMENT_GRADE=dev`; rejected otherwise. Default `false` |

Without `allow_local_urls` (or outside `DEPLOYMENT_GRADE=dev`), `base_url` and
every interface URL in an inline Agent Card must pass the safe-URL check, which
refuses localhost, private ranges, and metadata addresses. At request time the
same URLs are DNS-pinned and redirects are disabled. A session's network access
policy is also applied to the endpoint before and after the Agent Card is
resolved.

## `spawn_agent`

| Parameter | Required | Description |
|---|---|---|
| `instructions` | Yes | Instructions sent to the external agent |
| `target` | Yes | `{ "type": "external_a2a", "id": "<agent id>" }`. `target.external_agent_id` is a deprecated spelling of `id` |
| `mode` | No | `foreground` (default) or `background` |
| `wait_timeout_secs` | No | How long to wait for the remote task, 1 to 86400. Default 300 |
| `wake_on_completion` | No | Wake the session when a background run finishes. Default `true` |
| `result_schema` | No | JSON Schema for a required structured result from the external agent |

`message_schema` is not supported: a remote agent cannot call
`report_task_progress`, so passing one fails. Text results are truncated to
8192 characters.

`foreground` waits for the remote task and returns its reply. `background`
returns a `task_id` at once. When task tracking is available, each run is a session task of kind
`external_agent`, so `wait_task` collects the result, `message_task` answers a
remote task that asks for input, and `cancel_task` stops it.

## See also

- [A2A](/features/a2a/), serving agents over A2A and the delegation overview
- [Agent Handoff](/capabilities/agent-handoff/), delegation to first-party agents
- [Sub Agents](/capabilities/sub-agents/), the session task tools
