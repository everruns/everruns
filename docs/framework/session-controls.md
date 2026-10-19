---
title: Session Controls
description: Run slash commands, hot-install capabilities, switch model, reasoning effort and MCP servers, and query session context on a live Engine session.
---

A host that drives a `Session`, such as a terminal UI, a print mode or an ACP
server, needs more than send and wait. It runs slash commands, installs
extensions while the session is live, switches model or reasoning effort, adds
MCP servers, and reviews what the agent did. `Session` exposes those operations
directly. None of them hands out a runtime, starts a turn or writes to the
conversation history.

Stability: experimental.

## Commands

Capabilities contribute slash commands. Every frontend of one session sees the
same list, and `run_command` dispatches against it with the context the next
model call will use: the same provider, message filters, capability
instructions and filesystem.

```rust
use everruns::{Agent, Engine, OpenAI};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("You maintain this repository.")
    .provider(OpenAI::from_env()?)
    .model("gpt-6.1-sol")
    .capability("btw")
    .build()?;
let session = Engine::new().create(agent);

for command in session.commands().await? {
    println!("/{} {}", command.name, command.description);
}
// `/btw` answers a side question without interrupting the main task.
let output = session.run_command("/btw", Some("which files changed?")).await?;
println!("{}", output.message);
# Ok(())
# }
```

`run_command` runs out of band: no message is appended and no turn starts. It
is safe to call from a tool of the running turn, which is how a model-facing
`run_command` tool reaches the same registry. An unknown command returns
`SessionControlError::Rejected`.

## Hot-installed capabilities

Registering code and activating it are separate steps. `Engine::register_capability`
makes a trusted capability (written against the Core `Capability` contract from
[host services](/framework/host-services/)) resolvable for every session the
Engine runs. It changes no model surface until a session activates it.

```rust
use everruns::hosting::services::Capability;
use everruns::{CapabilityRef, Engine, Session, SessionControlError};

# async fn install(engine: &Engine, session: &Session, extension: impl Capability + 'static)
# -> Result<(), Box<dyn std::error::Error>> {
engine.register_capability(extension)?;
session
    .activate_capability(CapabilityRef::new("my_extension"))
    .await?;
// Its tools, instructions and commands are part of the next model call.
session.run_command("/my-command", None).await?;

session.deactivate_capability("my_extension").await?;
// Absent from the next model call; its commands are gone too.
# Ok(())
# }
```

Activation validates the configuration and dependencies before anything
changes, and repeating it is a no-op. A capability the Agent declares cannot be
deactivated at the session layer. Registration never overrides: an id or alias
the Framework catalog or another registration already serves is rejected.

The activation is the session's persisted capability overlay. Capability code
is never persisted, so after a restart register it again before resuming
sessions that use it, as you re-attach the Agent.

## Model, reasoning effort and MCP servers

```rust
use everruns::{McpServer, Model, ReasoningEffort, Session};

# async fn configure(session: &Session) -> Result<(), Box<dyn std::error::Error>> {
session.set_model("gpt-6-luna").await?;
session.set_reasoning_effort(Some(ReasoningEffort::High)).await?;
session
    .add_mcp_server(McpServer::http("docs", "https://mcp.example.com/mcp"))
    .await?;

// Unavailable selections fail and leave everything as it was.
assert!(session.set_model(Model::on("not-configured", "x")).await.is_err());
# Ok(())
# }
```

A plain model id selects a model of the Agent's provider; `Model::on` names a
provider the Agent or its host provider services already serve. A model value
that bundles its own provider is refused: a live selection never changes
credentials or provider code, and nothing falls back to another provider.
Changes apply from the next model call, including later calls of a turn that is
running. `reset_model` returns to the Agent's model, and `refresh_mcp`
rediscovers MCP tools after a credential refresh.

## Queries for host review

`command_context` returns the same projection as `inspect` (model, filtered
messages, tools, instructions and commands) but answers during an active turn
too. `complete_out_of_band` runs a tool-less completion over that context with
the session's provider and selections, for checks such as reviewing whether a
task was finished:

```rust
# async fn review(session: &everruns::Session) -> Result<(), everruns::SessionControlError> {
let verdict = session
    .complete_out_of_band(Some("Answer yes or no."), "Was the request completed?")
    .await?;
# let _ = verdict;
# Ok(())
# }
```

When the host owns the review policy and its token budget, `complete` sends a
request the host curates instead. The provider receives exactly its system
prompts and messages, with no tools. The session adds none of its own
instructions or conversation, and no message filters or model views apply.
What the session does contribute is what it owns: the provider and model its
next model call would use (live selections included), the Agent's call
settings, attachment resolution and provider errors.

```rust
use std::time::Duration;
use everruns::{CompletionRequest, ReasoningEffort};

# async fn review(session: &everruns::Session, evidence: String)
# -> Result<(), everruns::SessionControlError> {
let verdict = session
    .complete(
        CompletionRequest::new()
            .system_prompt("Decide whether the task is complete. Answer yes or no.")
            .user(evidence)
            .purpose("completion_review")
            .reasoning_effort(ReasoningEffort::Low)
            .timeout(Duration::from_secs(60)),
    )
    .await?;
# let _ = verdict;
# Ok(())
# }
```

To review with filtered session context, read `command_context` and reduce it
into your own bounded messages. A request carries 1 to 64 user or assistant
messages and up to 8 system prompts. System, tool-call and tool-result
messages are rejected.

- **Metadata.** `purpose` and `metadata` add provider metadata: up to 8
  entries, keys of lowercase ASCII letters, digits, `_`, `-` or `.`, and values
  up to 256 bytes. Keys the session stamps from its own authority
  (`session_id`, `harness_id`, `turn_id`, `exec_id`, `org_id`, `agent_id`,
  `model_id`) are rejected, not overwritten. When the host also sets request
  metadata for every model call, the request's keys win over the host's, and
  `session_id` always comes from the session.
- **Reasoning effort.** Without `reasoning_effort` the request uses the
  session's current selection. A per-request effort applies to that request
  only, and an effort the model's curated profile cannot honor is rejected.
- **No fallback.** A selected provider the session can no longer serve fails
  the request instead of reaching another provider.
- **Timeouts.** `timeout` ends the request with `SessionControlError::TimedOut`.
  Dropping the future abandons it as well.

`complete_out_of_band` remains the convenience form: it is a request whose
system prompts are the session's instructions plus yours, and whose messages
are the filtered conversation plus your prompt.

None of these append to history, start a turn, take execution ownership or
resume interrupted work, and all of them are answered during an active turn,
including from a tool of that turn. `history()` remains the bounded way to page
the canonical conversation.

## Where the state lives

| Setting | Coherent hosting (default Engine, `EngineBuilder::hosting`) | Custom backends and local profiles |
|---|---|---|
| Capability overlay | Committed through the hosting authority; durable | Persisted in the session record |
| Model and reasoning effort | Committed through the hosting authority; durable | Held by the Engine; re-apply after a restart |
| MCP servers | Held by the Engine; apply from the next activation | Held by the Engine; re-apply after a restart |
| Registered capability code | Held by the Engine; register again after a restart | Same |

MCP definitions can carry credentials, so they are never committed to session
state. On coherent hosting, commands run on the query runtime that inspection
uses; a change a command makes to the capability overlay is committed, and
other session-record writes are not retained.

## Related

- [Host services](/framework/host-services/) for capabilities and providers every session of an Agent starts with.
- [Session hosting](/framework/hosting/) for durable session authority under Engine.
