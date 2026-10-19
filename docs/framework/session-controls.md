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

Neither appends to history, takes execution ownership or resumes interrupted
work. `history()` remains the bounded way to page the canonical conversation.

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
