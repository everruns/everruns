# A2A Capability

<!-- Design Decisions:
  - V1 is outbound-only: Everruns agents can delegate to external A2A agents.
  - Do not expose Everruns agents over A2A by default.
  - Inbound A2A is an explicit future App channel, not a global agent endpoint.
  - Runtime surface is unified agent delegation: spawn_agent/get_agent_runs/wait_agent/message_agent/cancel_agent.
  - V1 stores external agents in capability config; future org-managed external_agents can replace this source.
  - Run state is registered as session_resources.kind = agent_run for visibility and wake-up.
  - Official Rust A2A SDK crates provide wire types, client, and integration-test server.
-->

## Abstract

The A2A capability lets an Everruns agent delegate work to configured external agents using the Agent2Agent protocol. It does not make Everruns agents publicly callable over A2A. The public/server side belongs to a later `a2a` App channel that must be explicitly enabled on a published App.

## Scope

V1 supports:

- AgentCard discovery from configured external A2A agents.
- JSON-RPC and HTTP+JSON transport negotiation through the official Rust A2A client.
- `spawn_agent` in `wait` or `background` mode.
- `wait_agent` for pending runs.
- `message_agent` for follow-up or `input_required` runs.
- `cancel_agent` for remote task cancellation.
- Wake-up by synthetic session message when a background run completes.
- Result persistence under `/.agent-runs/{agent_run_id}/result.json` when session files are available.

V1 does not support:

- Inbound A2A endpoint exposure.
- Arbitrary model-selected A2A URLs.
- Durable push webhook callbacks from remote A2A agents.
- Encrypted org-level external-agent credentials.

## Model

### Configured External Agents

V1 uses per-capability config:

```json
{
  "agents": [
    {
      "id": "research",
      "name": "Research Agent",
      "description": "External research agent",
      "base_url": "https://agent.example.com",
      "preferred_binding": "JSONRPC",
      "poll_interval_ms": 1000
    }
  ]
}
```

`base_url` resolves `/.well-known/agent-card.json`. `agent_card` may be provided inline for tests or deployments that cache cards externally.

`allow_local_urls` exists only for local integration tests and development. Production config should keep it false so `validate_safe_url` blocks localhost, private IP ranges, link-local addresses, and metadata endpoints.

### Future External Agent Registry

The config source should later become an org-scoped `external_agents` table:

- `id`, `org_id`, `name`, `description`
- `protocol = a2a`
- `base_url`, cached `agent_card`
- supported bindings and capabilities
- encrypted auth config
- lifecycle/status

The runtime tool contract should not change when this lands.

### Agent Runs

Every external task is represented as an `agent_run` session resource:

- `resource_id = agrun_*`
- `kind = agent_run`
- `status = active | completed | failed`
- metadata carries:
  - `kind = external_a2a`
  - `external_agent_id`
  - `remote_task_id`
  - `remote_context_id`
  - normalized run status
  - result/error summary
  - result artifact path
  - last remote task snapshot

This keeps external A2A and local subagent delegation visible through the same session-resource infrastructure.

Each run is additionally tracked as a session task (`kind = external_agent`)
with the full lifecycle, message thread, and `task.*` events — see
[`specs/session-tasks.md`](./session-tasks.md). The generic `message_task` /
`cancel_task` / `wait_task` tools work on agent runs via the
`ExternalAgentTaskExecutor`.

## Tools

### `spawn_agent`

Creates an external A2A run.

Parameters:

- `task` required
- `target.type = external_a2a`
- `target.external_agent_id`
- `mode = wait | background`
- `wait_timeout_secs`
- `wake_on_completion`

`mode = wait` blocks until the remote task reaches a terminal state or timeout. `mode = background` returns an `agent_run_id` immediately and monitors completion in a detached task.

### `get_agent_runs`

Lists all session `agent_run` resources or returns one run by `agent_run_id`.

### `wait_agent`

Polls the remote A2A task by `remote_task_id` until terminal state or timeout.

### `message_agent`

Sends follow-up input using the saved remote `taskId` and `contextId`.

### `cancel_agent`

Calls A2A task cancellation and updates the local run state from the returned task.

## A2A Mapping

| Everruns | A2A |
|---|---|
| `agent_run_id` | local handle |
| `remote_task_id` | A2A `Task.id` |
| `remote_context_id` | A2A `Task.contextId` |
| `task` / follow-up message | A2A `Message` with `ROLE_USER` |
| result summary | first text artifact, else status message text |
| `input_required` | non-terminal interrupted run |
| `auth_required` | non-terminal interrupted run |
| `cancel_agent` | A2A `CancelTask` |

## Wake-Up

When `wake_on_completion = true`, background completion injects a synthetic user message into the parent session:

```text
External agent run completed.
- run_id: agrun_...
- agent: Research Agent
- status: completed
- result_path: /.agent-runs/agrun_.../result.json
- summary: ...
```

This matches existing background-run behavior and avoids adding inbound A2A or push-webhook requirements in V1.

## Security

Relevant threat categories: `TM-API`, `TM-TOOL`, `TM-AGENT`, `TM-DOS`.

Required mitigations:

- The model cannot provide arbitrary A2A URLs; it chooses a configured `external_agent_id`.
- Configured URLs pass `validate_safe_url` unless explicitly marked `allow_local_urls`.
- Remote IDs are opaque strings and are only used with the configured agent that produced them.
- Result snapshots are bounded by normal tool-result and session-resource limits.
- Secret-bearing auth should move to encrypted org-managed external-agent config before production use with private agents.
- Inbound A2A must be a separate App channel and follow `specs/public-endpoints.md`.

## Testing

Integration tests use the official Rust `a2a-server-lf` crate to start a real local A2A agent over JSON-RPC. Tests cover:

- AgentCard discovery.
- `spawn_agent` wait mode.
- Background mode plus `wait_agent`.
- Local URL blocking unless `allow_local_urls` is set.
