---
title: Moving to Platform or Cloud
description: What carries over when a Framework agent moves to a self-hosted Everruns Platform or Everruns Cloud, what has to be rebuilt, and what stays behind.
---

The Framework and the Platform run the same runtime, so an agent's behavior
moves with it: instructions, capabilities and their configuration, files, MCP
servers, and loop settings all have Platform counterparts. What does not move
is code. A Platform agent is configuration stored by the server, so anything
your Framework app expresses as Rust (function tools, custom capabilities,
lifecycle closures, an approver) is rebuilt with a Platform mechanism instead.

Everruns Cloud is the hosted Platform. Everything here applies to both, except
that Cloud runs the production deployment grade: capabilities marked dev-only,
such as [Agent Handoff](/capabilities/agent-handoff/), are not offered there.

## What transfers

| Framework | Platform agent field |
|---|---|
| `.name("support")` | `name`. Must be lowercase letters, digits, and hyphens, unique in the organization |
| `.instructions(...)` | `system_prompt` |
| `.model("gpt-5.6-terra")` | `default_model_id`, the ID of a model configured for the organization. A model name string is not accepted |
| `.capability(...)` | `capabilities`, the same `{ "ref": ..., "config": ... }` shape |
| `.initial_file(...)`, `.file(...)`, `.readonly_file(...)` | `initial_files` |
| `.mcp_server(...)` for a remote server | `mcpServers` |
| `.max_iterations(n)` | `max_iterations` |
| `.parallel_tool_calls(b)` | `parallel_tool_calls` |
| A `Harness` | A Platform harness (`harness_id` or `harness_name`). `Harness::generic()` uses the same capability list as the Platform's built-in `generic` harness |

`CapabilityRef` serializes as `{"ref": id, "config": {...}}`, which is the
format of a Platform agent's `capabilities` entries, so a capability list can
be copied across as JSON. Check each ID against the
[capability index](/capabilities/): a few Framework capabilities are
Framework-only (for example host shell), and a capability whose page lists only
Platform or Cloud adds features the Framework never had.

To create the agent, write it as a file and import it with
`POST /v1/agents/import`, the CLI, or an SDK. See
[Define agents as files](/how-to/define-agents-as-files/).

```rust
use everruns::{Agent, CapabilityRef, OpenAI};
use serde_json::json;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .name("support")
    .instructions("Answer support questions.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .capability("current_time")
    .capability(CapabilityRef::new("loop_detection").config(json!({ "threshold": 4 })))
    .build()?;
# let _ = agent;
# Ok(())
# }
```

becomes:

```markdown
---
name: support
capabilities:
  - current_time
  - ref: loop_detection
    config:
      threshold: 4
---
Answer support questions.
```

Pick the default model in the Platform after import, from the models the
organization has configured.

## What has to be rebuilt

| Framework | On the Platform |
|---|---|
| Function tools (`.tool(...)`, `#[everruns::tool]`) | Client-side tools in the agent's `tools` field, which the model calls and your client executes when it receives `tool.call_requested`; or a remote MCP server that exposes the same tools |
| Custom capabilities written with `everruns::capability` | Built-in capabilities, a remote MCP server, or client-side tools. The Platform does not load application code |
| Lifecycle hooks (`.on_turn_start`, `.on_tool_start`, ...) | [User Hooks](/capabilities/user-hooks/), which run shell commands at lifecycle and tool events |
| `.approver(...)` | [Tool Approval](/capabilities/tool-approval/), answered by a person in the UI or by your client over the API |
| `.ask_user(...)` | [Ask User](/capabilities/ask-user/), answered by the session's user |
| stdio MCP servers | Not supported. The Platform connects only to remote MCP servers |
| `OpenAI::from_env()` and other `from_env` credentials | Provider keys set in the organization's settings and stored encrypted. Drivers do not read them through `from_env` on the server |

[`examples/client_side_tools.sh`](https://github.com/everruns/everruns/blob/main/examples/client_side_tools.sh)
shows the client-side tool loop over the API.

## What does not transfer

- **Sessions and history.** There is no session import. A Framework session's
  event log, whether in memory or under a `LocalConfig` data directory, stays
  with the Framework app. Platform sessions start fresh; a Platform session can
  be exported (`GET /v1/sessions/{id}/export`) but not the reverse.
- **Workspace files.** A local workspace directory is not copied. Put files the
  agent needs at start in `initial_files`.
- **Application wiring.** Event listeners, `Engine` configuration, and your own
  HTTP layer have no Platform equivalent. Platform sessions are driven through
  the [API](/api/), SDKs, CLI, or UI, and observed through its
  [observability](/observability/) exporters.

## Keeping both

The same capability IDs and configuration work in both places, so an agent can
be developed in the Framework with simulated models and tests, then registered
on the Platform. `CapabilityRef` implements `Deserialize`, so one JSON file of
capability entries can feed both the Framework builder and the Platform agent.

## See also

- [Portable and hosted capabilities](/framework/capability-boundaries/)
- [Deploying a Framework app](/framework/deployment/)
- [Define agents as files](/how-to/define-agents-as-files/)
