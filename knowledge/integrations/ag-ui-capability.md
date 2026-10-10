---
type: Specification
title: "AG-UI Capability"
description: "AG-UI outbound delegation capability: configured external AG-UI agents as spawn_agent targets backed by session tasks."
tags:
  - everruns
  - integrations
---
# AG-UI Capability

<!-- Design Decisions:
  - Outbound only, and the mirror of the A2A capability: external agents come from the
    capability config, the model picks a configured id, never a URL.
  - Same runtime surface as every delegation: spawn_agent plus the generic session_tasks tools.
  - Own task kind (external_ag_ui), because the executor registry resolves one executor per kind.
  - The SSE stream is the only handle on a remote run (AG-UI has no "get run"), so a stream
    lost with its worker is failed as orphaned rather than re-attached.
  - A remote interrupt parks the task in awaiting_input; message_task answers it with the
    resuming run on the same thread.
  - The bearer token is a session secret referenced by name; nothing secret is stored in config.
-->

## Abstract

The `ag_ui_delegation` capability lets an Everruns agent delegate work to configured external
agents that speak [AG-UI 1.0](ag-ui.md). Each delegation is a session task: the agent starts it
with `spawn_agent`, and `wait_task`, `message_task` and `cancel_task` work on it unchanged. The
remote agent's text streams back as the task result, a remote interrupt becomes the task
waiting for input, and cancelling closes the stream.

It is the client side of the protocol. The server side, exposing an Everruns agent over AG-UI,
is the [AG-UI Channel](ag-ui.md). The [A2A Capability](a2a-capability.md) is the same contract
over A2A.

## Packaging

`everruns-capabilities`'s `ag-ui` feature gates the
[`ag_ui_delegation`](../../crates/capabilities/src/capabilities/ag_ui_delegation/mod.rs) module and
its registration, and enables core’s `ag-ui-client` feature. The product build
(`everruns-server`, `everruns-worker`) enables it. Registration also follows the
deployment's `FEATURE_AGENT_DELEGATION` decision, like the other delegation capabilities;
there is no org flag, since adding the capability to an agent is the opt-in.

The target type and the task kind are both `external_ag_ui`, defined ungated in
`everruns-core` next to the A2A ones, so the unified `spawn_agent` tool can advertise the
target and refuse `lifetime = "detached"` and `message_schema` for it exactly as it does
for `external_a2a`.

## Configuration

```json
{
  "agents": [
    {
      "id": "reports",
      "name": "Reports Agent",
      "description": "Writes quarterly summaries",
      "url": "https://agents.example.com/reports",
      "bearer_token_secret": "REPORTS_AGENT_TOKEN",
      "headers": { "x-tenant": "acme" }
    }
  ]
}
```

- `url` is the AG-UI endpoint. A run is a POST of `RunAgentInput` answered by an SSE stream.
- `bearer_token_secret` names a session secret. Its value is read when each run starts, sent
  as `Authorization: Bearer`, and never stored with the run, the task or the tool result.
- `headers` are non-secret. `Authorization`, `Proxy-Authorization` and `Cookie` are refused.
- `allow_local_urls` is the development escape hatch for local agents, as in A2A.

The schema and its validation are in
[`ag_ui_delegation/mod.rs`](../../crates/capabilities/src/capabilities/ag_ui_delegation/mod.rs).

## Runtime contract

| Everruns | AG-UI |
|---|---|
| `spawn_agent` with `target.type = "external_ag_ui"` | first run on a new thread, the instructions as the user message |
| `agent_run_id` | local handle, record at `agent_run:{run_id}` in session storage |
| task result (`summary`) | the agent's own assistant text (`RunResult::text`), bounded |
| `result_schema` | validated against the `RUN_FINISHED` `result` value |
| task `awaiting_input` | `RUN_FINISHED` with the `interrupt` outcome; the prompt lists the interrupts |
| `message_task` while awaiting input | a resuming run: same `threadId`, `parentRunId`, the conversation so far, and a `resume` entry for every open interrupt |
| `cancel_task` | the stream is closed |
| task `failed` | `RUN_ERROR`, a protocol violation, an HTTP error, or the timeout |

A `message_task` answer that is a JSON object keyed by open interrupt ids answers each listed
interrupt and abandons the rest; any other answer resolves every open interrupt with that
payload. Either way the resume satisfies the 1.0 coverage rule. A message to a run that is not
waiting for input is refused: an AG-UI run takes no input mid-stream, and a finished delegation
is continued by spawning a new one.

Background runs use the `on_activity` wake policy, so the parent wakes when the remote agent
asks for input as well as when it finishes. The task transition is the wake; no outbound task message is posted on
top, which under that policy would wake the parent twice. A resuming run keeps the stream
deadline (`wait_timeout_secs`) the spawn asked for, and a run that reaches it is closed and
failed: the stream is the only handle, so there is nothing to leave running.

Cancellation reaches the stream two ways: directly when the stream runs in the same process,
and through the task's `cancel_requested_at` otherwise, which the streaming loop checks every
second. The loop heartbeats the task, so a stream lost with its worker is reaped as orphaned.

## Security

- **Egress (TM-AGENT-030).** URLs come only from the admin-set config. Every request, including
  each resume, is checked against the session's merged network ACL, refuses private and
  metadata addresses with the resolved address pinned, and follows no redirects.
- **Untrusted stream content (TM-AGENT-031).** The stream is validated by the consumer
  pipeline (1.0 sequencing rules, bounded event size). Stored text, history and interrupts are
  bounded; remote text reaches the model only as a tool result or a task summary.
- **Credentials (TM-AGENT-032).** The bearer token lives in the session secret store and is sent
  as a sensitive header. The config holds only the secret's name.

The capability is `RiskLevel::High`, so only admins can assign it.

## Testing

[`ag_ui_delegation/tests.rs`](../../crates/capabilities/src/capabilities/ag_ui_delegation/tests.rs)
runs the capability against a local mock AG-UI agent: foreground and background runs, the
generic task tools, interrupt and resume, cancel closing the stream, remote errors, HTTP
errors, protocol violations, `result_schema`, the missing-secret and ACL refusals, and config validation.
