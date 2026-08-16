---
type: Specification
title: "A2A Capability"
description: "A2A outbound delegation capability."
tags:
  - everruns
  - integrations
---
# A2A Capability

<!-- Design Decisions:
  - V1 is outbound-only: Everruns agents can delegate to external A2A agents.
  - Do not expose Everruns agents over A2A by default.
  - Inbound A2A is an explicit future App channel, not a global agent endpoint.
  - Runtime surface is unified agent delegation: spawn_agent + generic session_tasks tools.
  - V1 stores external agents in capability config; future org-managed external_agents can replace this source.
  - Run state is registered as session_resources.kind = agent_run for visibility and wake-up.
  - Official Rust A2A SDK crates provide wire types, client, and integration-test server.
  - get_agent_runs, wait_agent, message_agent, cancel_agent retired in favour of generic tools (see migration below).
-->

## Abstract

The A2A capability lets an Everruns agent delegate work to configured external agents using the Agent2Agent protocol. It does not make Everruns agents publicly callable over A2A. The public/server side belongs to a later `a2a` App channel that must be explicitly enabled on a published App.

## Scope

V1 supports:

- AgentCard discovery from configured external A2A agents.
- JSON-RPC and HTTP+JSON transport negotiation through the official Rust A2A client.
- `spawn_agent` in `foreground` or `background` mode, using the same execution
  vocabulary as local delegation providers.
- `wait_task` for pending runs (generic session task tool).
- `message_task` for follow-up or `input_required` runs (generic session task tool).
- `cancel_task` for remote task cancellation (generic session task tool).
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

The ARD client capability (`resource_discovery`, [`crates/ard/SPEC.md`](../../crates/ard/SPEC.md)) is one realization of this discovery source, generalized to MCP servers as well as A2A agents: it discovers external capabilities from ARD registries and attaches them mid-session. ARD-attached A2A agents are merged into this capability's `agents` config during turn-context assembly (`everruns_core::ard_attachment::apply_session_attachments`), so they flow through the existing `spawn_agent` path unchanged.

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
with the full lifecycle, message thread, and `task.*` events, see
[`knowledge/runtime-resources/session-tasks.md`](../runtime-resources/session-tasks.md). The generic `list_tasks`,
`get_task`, `message_task`, `cancel_task`, and `wait_task` tools work on
agent runs via the `ExternalAgentTaskExecutor`.

## Tools

### `spawn_agent`

Creates an external A2A run. Returns an `agent_run_id` and `task_id`.
When other known delegation providers are active, external A2A is advertised as
the `external_a2a` target in the shared `spawn_agent` dispatcher rather than as
a competing tool definition.

Parameters:

- `instructions` required
- `target.type = external_a2a`
- `target.id` (the A2A provider also accepts its provider-specific
  `target.external_agent_id` spelling)
- `mode = foreground | background`
- `wait_timeout_secs`
- `wake_on_completion`
- `result_schema`: optional JSON Schema for a required terminal structured
  artifact. The first A2A data part is validated against it.
- `message_schema` is unsupported for remote A2A agents and causes an explicit
  spawn error; Everruns cannot inject `report_task_progress` into a remote
  agent.

`mode = foreground` blocks until the remote task reaches a terminal state or
timeout. `mode = background` returns an `agent_run_id` immediately and monitors
completion in a detached task. The provider parses these values directly; the
shared dispatcher does not translate provider-specific mode dialects.

When `result_schema` is present, a completed remote task succeeds only if its
first structured data artifact conforms. A valid value is written to
`/.tasks/{task_id}/result.json` and recorded as the session-task result. Missing
structured data fails with `error.kind = "no_result"`; invalid data fails with
`error.kind = "schema_mismatch"`. Runs without a schema retain the legacy text
summary and `/.agent-runs/{run_id}/result.json` snapshot behavior.

### Monitoring and steering agent runs

Use the generic `session_tasks` tools after spawning. The `task_id` is returned by `spawn_agent`.

| Tool | Description |
|------|-------------|
| `list_tasks` with `kind: "external_agent"` | List all agent run tasks |
| `get_task` | Get detailed status for a specific run |
| `message_task` | Send follow-up input using the saved remote taskId/contextId |
| `cancel_task` | Call A2A task cancellation and update local run state |
| `wait_task` | Poll until terminal state or timeout |

### Retired tools (removed)

The following per-kind tools were retired in favour of the generic tools above:

| Retired | Replacement |
|---------|-------------|
| `get_agent_runs` | `list_tasks(kind="external_agent")` + `get_task` |
| `wait_agent` | `wait_task` |
| `message_agent` | `message_task` |
| `cancel_agent` | `cancel_task` |

## A2A Mapping

| Everruns | A2A |
|---|---|
| `agent_run_id` | local handle |
| `remote_task_id` | A2A `Task.id` |
| `remote_context_id` | A2A `Task.contextId` |
| `instructions` / follow-up message | A2A `Message` with `ROLE_USER` |
| result summary | first text artifact, else status message text |
| schema-bound machine result | first structured data artifact |
| `input_required` | non-terminal interrupted run |
| `auth_required` | non-terminal interrupted run |
| `cancel_task` | A2A `CancelTask` |

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
- Result snapshots are bounded by normal tool-result and session-task limits;
  schema-bound data is treated as untrusted and validated before persistence.
- Secret-bearing auth should move to encrypted org-managed external-agent config before production use with private agents.
- Inbound A2A must be a separate App channel and follow `knowledge/execution/public-endpoints.md`.

## Testing

Integration tests use the official Rust `a2a-server-lf` crate to start a real local A2A agent over JSON-RPC. Tests cover:

- AgentCard discovery.
- `spawn_agent` foreground mode and terminal/timeout parity with the former
  blocking behavior.
- Structured artifact validation and task-result persistence.
- Schema mismatch settlement and explicit `message_schema` rejection.
- Background mode (background spawn).
- Local URL blocking unless `allow_local_urls` is set.
