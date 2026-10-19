---
title: Host Services
description: Compose an embedding host's own capabilities, provider resolution, retry budgets, MCP credentials and observers into Engine sessions.
---

An embedding host often owns more than an agent definition: a curated set of
capability implementations with guards and context controls, its own provider
and credential resolution, provider recovery budgets, refreshing MCP tokens,
and event observers. `everruns::hosting::services` composes those into the
runtimes `Engine` builds, so the host keeps its behavior and still uses
Engine's sessions, durability and turn runner.

Stability: experimental, together with [session hosting](/framework/hosting/).

## What host services can and cannot change

`HostServices` is an immutable value attached to an Agent with
`AgentBuilder::host_services`. Engine applies it every time it constructs that
Agent's runtime: for new sessions, resumption, durable activations and context
inspection.

| Host services contribute | Engine keeps owning |
|---|---|
| Capabilities written against the Core `Capability` contract, with pre-tool hooks, model views, slash commands and instructions | Session identity, admission, leases and the turn runner |
| Provider resolution through a host `ProviderStore` and `DriverRegistry` | The Environment-selected session filesystem and its containment |
| Provider retry and stream-stall budgets | Canonical history: observers only observe |
| Call-time MCP credentials | Framework capability ids, aliases and tool names |

There is no callback that receives a runtime builder, and no runtime handle to
drive. A host capability whose id, alias or tool name collides with a Framework
capability or another input fails `AgentBuilder::build`, so a host cannot
replace, for example, the filesystem capability by registering the same id.
Capability tools reach files only through the session filesystem in their
`ToolContext`, which is the one the session's Environment selected.

## Add a host capability

Implement `Capability` for the host's policy. This one blocks one tool before it
runs and adds a reminder to every model call:

```rust
use std::sync::Arc;
use everruns::hosting::services::{
    Capability, HostServices, ModelViewContext, ModelViewProvider, PolicyGate,
    PreToolUseDecision, PreToolUseHook, RuntimeMessage, ToolCall, ToolContext,
    ToolDefinition, async_trait,
};
use everruns::{Agent, Engine, OpenAI};
use serde_json::Value;

struct NoForcePush;

#[async_trait]
impl PreToolUseHook for NoForcePush {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _definition: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        if tool_call.name == "git_push" && tool_call.arguments["force"] == true {
            return PreToolUseDecision::Block {
                tool_call,
                reason: "force pushes are disabled by policy".into(),
                user_message: None,
            };
        }
        PreToolUseDecision::Continue(tool_call)
    }
}

struct Reminder;

impl ModelViewProvider for Reminder {
    fn apply_model_view(
        &self,
        mut messages: Vec<RuntimeMessage>,
        _config: &Value,
        _context: &ModelViewContext<'_>,
    ) -> Vec<RuntimeMessage> {
        messages.push(RuntimeMessage::system("Keep changes reviewable."));
        messages
    }
}

struct RepoPolicy;

impl Capability for RepoPolicy {
    fn id(&self) -> &str { "repo_policy" }
    fn name(&self) -> &str { "Repository policy" }
    fn description(&self) -> &str { "Guards repository operations." }
    fn system_prompt_addition(&self) -> Option<&str> {
        Some("Never rewrite published history.")
    }
    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn PreToolUseHook>> {
        vec![Arc::new(PolicyGate(NoForcePush))]
    }
    fn model_view_provider(&self) -> Option<Arc<dyn ModelViewProvider>> {
        Some(Arc::new(Reminder))
    }
}

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("You maintain this repository.")
    .provider(OpenAI::from_env()?)
    .model("gpt-6.1-sol")
    .host_services(HostServices::new().capability(RepoPolicy))
    .build()?;
let session = Engine::new().create(agent);
let context = session.inspect().await?;
assert!(context.instructions.contains("Never rewrite published history."));
# Ok(())
# }
```

`PolicyGate` marks the hook as a policy decision, so it runs after any hook that
rewrites arguments and decides on the call that will execute. A blocked call
never reaches its tool; the model receives the reason as the call's result.
Commands a capability contributes appear in `SessionContext::commands`.

`HostServices::capability` registers and activates. `HostServices::register`
only registers, and the Agent activates it with an ordinary
`.capability("repo_policy")` reference or through its Harness.

## Resolve providers through the host

With `HostProviders`, the host's `ProviderStore` owns endpoints and credentials
and the Agent names its model with `Model::on(provider_key, model_id)`. The
store is asked under the calling session's identity on every provider call, so
rotated credentials and provider metadata reach the next call without
rebuilding anything.

```rust
use everruns::hosting::services::{HostProviders, HostServices, ProviderStore};
use everruns::{Agent, DriverRegistry, Model};

# fn build(store: impl ProviderStore + 'static) -> Result<(), everruns::BuildError> {
let mut drivers = DriverRegistry::new();
everruns::drivers::openai::register_driver(&mut drivers);
let agent = Agent::builder()
    .instructions("You are concise.")
    .model(Model::on("team-openai", "gpt-6.1-sol"))
    .host_services(HostServices::new().providers(HostProviders::new(store, drivers)))
    .build()?;
# let _ = agent;
# Ok(())
# }
```

Selection is never implicit. Building fails with `ConflictingProviders` when a
host-resolved model also carries an Agent provider, or when `Model::on` names a
different provider than `.provider(..)`. A provider key the host cannot serve
fails runtime construction before any model call, rather than falling back to
another driver.

## Budgets, MCP credentials and observers

```rust
use std::time::Duration;
use everruns::hosting::services::{
    HostEvent, HostEventListener, HostServices, LlmRetryConfig, McpAuthProvider,
    McpAuthRequest, McpAuthResult, McpCredential, async_trait,
};
use everruns::Engine;

struct Vault;

#[async_trait]
impl McpAuthProvider for Vault {
    async fn authorization(
        &self,
        request: &McpAuthRequest<'_>,
    ) -> McpAuthResult<Option<McpCredential>> {
        // Called before every discovery and tool call: return the current token.
        Ok(Some(McpCredential::bearer(format!("token-for-{}", request.server_name))))
    }
}

struct Audit;

#[async_trait]
impl HostEventListener for Audit {
    async fn on_event(&self, event: &HostEvent) {
        println!("{} {:?}", event.event_type, event.sequence);
    }
}

let services = HostServices::new()
    .provider_retry(LlmRetryConfig { max_retries: 4, ..LlmRetryConfig::default() })
    .provider_stall_timeout(Duration::from_secs(45))
    .mcp_auth(Vault);
let engine = Engine::builder().host_listener(Audit).build();
# let _ = (services, engine);
```

Host listeners receive the same committed canonical envelopes as
`EngineBuilder::listener`, in Core event form, on their own bounded queue. A
slow, full or failing listener cannot fail a turn or delay other listeners, and
nothing a listener does is written to session history. Drops are counted in
`Engine::observer_stats`. `McpAuthProvider` requires the `mcp` feature.

## Related

- [Advanced capabilities](/framework/advanced-capabilities/) for typed capability packages that need no Core contract.
- [Session hosting](/framework/hosting/) for durable session authority under Engine.
- [Custom backends](/framework/custom-backends/) when the host itself replaces execution components.
