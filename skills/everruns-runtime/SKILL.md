---
name: everruns-runtime
description: Embed the Everruns agent engine directly in a Rust process with the everruns-runtime crate — run harnesses in-process with no PostgreSQL, server, or worker. Use when building local agentic apps (coding agents, personal agents, CLIs), writing in-process agent tests, registering custom capabilities/drivers/backends, or wiring optional provider and integration crates (OpenAI, Anthropic, Daytona, E2B, …).
license: MIT
compatibility: Rust 1.94+ (edition 2024)
metadata:
  category: agent-runtime
  version: "1.0"
  homepage: https://everruns.com
  docs: https://docs.everruns.com/features/runtime/
  api-reference: https://docs.rs/everruns-runtime
  repository: https://github.com/everruns/everruns
---

# everruns-runtime

> Embed Everruns agents directly in your Rust process — no server, worker, or database required.

`everruns-runtime` is the public, in-process entrypoint for [Everruns](https://everruns.com),
the open-source durable agentic harness engine. It runs an Everruns **harness**
entirely inside your own binary, using the same `input → reason → act` loop as
the worker- and server-backed runtime, but **in-memory by default** — no
PostgreSQL, no control-plane server, no gRPC worker boundary.

It is the Rust-native counterpart to the [SDKs](https://docs.everruns.com/features/sdk/):
where the SDKs talk to a *running* Everruns server over HTTP, the runtime **is**
the engine, linked straight into your code.

## When to Use

Use this skill when you want to:

- Run an agent harness in-process with no external services
- Build a local agentic application — coding agent, personal agent, CLI, daemon
- Write fast, deterministic in-process agent tests (with the built-in `llmsim` driver)
- Register your own **capabilities**, LLM **drivers**, and tools
- Provide custom backend stores (files, messages, sessions, …) instead of the in-memory defaults
- Inspect the assembled turn context before execution
- Pull in optional **provider** or **integration** crates (OpenAI, Anthropic, Daytona, E2B, …)

If instead you want to call a *hosted* Everruns deployment over HTTP, use an
[SDK](https://docs.everruns.com/features/sdk/), not this crate.

## Install

Requires Rust **1.94+** (edition 2024).

```bash
cargo add everruns-runtime
# Shared types (capabilities, drivers, models) live in everruns-core:
cargo add everruns-core
```

The runtime ships in-memory by default, so the smallest useful host is a single
`build().await?` away.

## Quickstart

Build an in-process runtime with an in-memory backend and the deterministic LLM
simulator (no *real* API key needed — `llmsim` ignores it, so any placeholder
works), then run a turn:

```rust
use everruns_core::{
    CapabilityRegistry, DriverId, DriverRegistry, PlatformDefinition, ResolvedModel,
};
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_runtime::InProcessRuntimeBuilder;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let platform = PlatformDefinition::new(CapabilityRegistry::new(), DriverRegistry::new());

let runtime = InProcessRuntimeBuilder::new()
    .platform_definition(platform)
    .llm_sim(LlmSimConfig::fixed("hello from everruns-runtime"))
    .default_model(ResolvedModel {
        model: "llmsim-model".into(),
        provider_type: DriverId::LlmSim,
        api_key: Some("fake-key".into()),
        base_url: None,
        provider_metadata: None,
    })
    .single_session(|s| {
        s.harness("assistant", "You are a concise, helpful assistant.")
            .agent("assistant-agent", "Answer in one short sentence.")
            .session_title("Quickstart")
    })
    .build()
    .await?;

let session_id = runtime.default_session_id().expect("single_session id");
let result = runtime.run_text_turn(session_id, "Say hi.").await?;
println!("{}", result.response);
# Ok(())
# }
```

`single_session(...)` is the compact path for the common one-harness,
one-agent, one-session shape; it records the generated id for
`default_session_id()`.

## Talk to a real model provider

A real provider needs **two** things: the provider's **driver must be
registered** in the `DriverRegistry`, *and* `default_model` must select it.
Setting `default_model` alone is not enough — the runtime resolves
`provider_type` against the registry at turn time.

```bash
cargo add everruns-openai
```

```rust
use everruns_core::driver_registry::DriverRegistry;
use everruns_core::{CapabilityRegistry, DriverId, PlatformDefinition, ResolvedModel};
use everruns_runtime::InProcessRuntimeBuilder;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
// 1. Register the OpenAI driver so `DriverId::OpenAI` resolves.
let mut drivers = DriverRegistry::new();
everruns_openai::register_driver(&mut drivers);

// 2. Build the platform from that registry.
let platform = PlatformDefinition::new(CapabilityRegistry::new(), drivers);

// 3. Point the runtime's default model at OpenAI.
let runtime = InProcessRuntimeBuilder::new()
    .platform_definition(platform)
    .default_model(ResolvedModel {
        model: "gpt-5.5".into(),
        provider_type: DriverId::OpenAI,
        api_key: Some(std::env::var("OPENAI_API_KEY")?),
        base_url: None,
        provider_metadata: None,
    })
    .single_session(|s| {
        s.harness("assistant", "You are a concise, helpful assistant.")
            .agent("assistant-agent", "Answer in one short sentence.")
            .session_title("OpenAI Session")
    })
    .build()
    .await?;
# Ok(())
# }
```

Swap `everruns-openai` for any provider crate, matching the crate to its
`DriverId` (`everruns-anthropic` → `DriverId::Anthropic`, etc. — see
[`references/crates.md`](references/crates.md)). `everruns-openai::register_driver`
actually registers three drivers — `DriverId::OpenAI` (Responses API),
`DriverId::AzureOpenAI` (Azure OpenAI Responses API), and
`DriverId::OpenAICompletions` (Chat Completions API) — so select the one matching
your deployment. To reach an OpenAI-compatible endpoint (a local gateway, Ollama,
…), set `base_url` and pick the `DriverId` matching the API the server speaks.

## Register capabilities (tools)

Capabilities are the tools and behaviors an agent can use. Register them on a
`CapabilityRegistry`, then reference them by name on a harness:

```rust,ignore
let mut capabilities = CapabilityRegistry::new();
capabilities.register(FileSystemCapability);   // from everruns-core
capabilities.register(DuckDuckGoCapability);   // from everruns-integrations-duckduckgo

let platform = PlatformDefinition::new(capabilities, drivers);
// ...
.single_session(|s| {
    s.harness("coder", "You are a coding agent.")
        .with_capability("session_file_system")
        .with_capability("duckduckgo")
        .agent("coder-agent", "Use tools when they help.")
})
```

Built-in capabilities live in `everruns-core`; optional ones ship as separate
**integration crates** (see below). Active capabilities are resolved from the
effective harness/agent/session overlay, not the full platform registry.

## Optional crates (providers, integrations, backends)

The runtime is provider- and tool-neutral; you pull in only what you use. The
full catalog with crate names, `DriverId`s, and capability names is in
[`references/crates.md`](references/crates.md). Highlights:

- **LLM providers**: `everruns-openai` (OpenAI + Azure OpenAI + Chat
  Completions), `everruns-anthropic`, `everruns-gemini`, `everruns-openrouter`,
  `everruns-fireworks`, `everruns-mai`, `everruns-bedrock`.
- **Sandbox / compute integrations**: `everruns-integrations-daytona` (Daytona
  cloud sandbox), `everruns-integrations-e2b` (E2B), `everruns-integrations-docker`,
  `everruns-integrations-deno`, `everruns-integrations-sprites`,
  `everruns-integrations-cursor`.
- **Search / web integrations**: `everruns-integrations-duckduckgo` (no key),
  `everruns-integrations-brave-search`, `everruns-integrations-parallel`,
  `everruns-integrations-browserless`, `everruns-integrations-ard`.
- **Durable local backends**: `everruns-local` — SQLite-backed,
  restart-survivable stores for single-process hosts (task registry, schedules,
  platform store).

> **Daytona example**: add `everruns-integrations-daytona`, register
> `DaytonaCapability`, and supply a credential source via
> `RuntimeBackends::with_connection_resolver(...)` so the connection-aware tool
> resolves the user's Daytona token at execution time. See
> [`references/crates.md`](references/crates.md).

## Inspect the turn context

`load_context(session_id)` returns the exact merged context the reason phase
will use — before the first turn or after messages already exist:

```rust,ignore
let context = runtime.load_context(session_id).await?;
println!("messages = {}", context.messages.len());
println!("model    = {}", context.runtime_agent.model);
println!("tools    = {}", context.runtime_agent.tools.len());
```

The context bundles the harness chain, optional agent, session, filtered message
history, resolved model, and the assembled `RuntimeAgent`.

## Custom backends

The runtime ships in-memory defaults, but public extension traits let you supply
your own stores via `RuntimeBackends`; any store you don't override stays
in-memory:

```rust,ignore
use everruns_runtime::{InProcessRuntimeBuilder, RuntimeBackends};

let runtime = InProcessRuntimeBuilder::new()
    .backends(
        RuntimeBackends::in_memory()
            .with_message_store(my_message_store)
            .with_event_bus(my_event_bus),
    )
    .build()
    .await?;
```

Overridable stores: `RuntimeHarnessStore`, `RuntimeAgentStore`,
`RuntimeSessionStore`, `RuntimeMessageStore`, `RuntimeProviderStore`, and
`EventBus` (extends `EventEmitter`). Session files are selected through the
platform's `SessionFileSystemFactory` (`InMemory...`, `RealDisk...`, or custom).

## Local MCP servers

MCP server tools are first-class runtime tools. Local-process (stdio) MCP
servers are gated behind the `mcp-stdio` feature, off by default:

```bash
cargo add everruns-runtime --features mcp-stdio
```

## Domain language

A short glossary — the full version with relationships is in
[`references/glossary.md`](references/glossary.md):

- **Harness** — the reusable agent template: system prompt + capabilities +
  config. Harnesses can chain (parent → child overlay).
- **Agent** — a configuration overlay on a harness (instructions, model, max
  iterations) that composes into a runtime agent.
- **Session** — a single conversation/run; holds message history and workspace.
- **Capability** — a registered tool or behavior (e.g. `session_file_system`,
  `duckduckgo`, `infinity_context`).
- **Driver** — an LLM provider implementation, keyed by `DriverId`.
- **PlatformDefinition** — the shared bundle (capabilities + drivers + harness
  templates + filesystem factory) consumed by runtime, server, and worker alike.
- **Turn** — one `input → reason → act` cycle.
- **Atom** — the shared core execution units (`InputAtom`, `ReasonAtom`,
  `ActAtom`) the runtime and worker both run.

## Runnable examples

In-tree examples that depend on the public crate exactly as an external embedder
would:

```bash
cargo run -p everruns-runtime --example in_process_runtime
cargo run -p everruns-runtime --example inspect_context
cargo run -p everruns-runtime --example openai_runtime          # needs OPENAI_API_KEY
cargo run -p everruns-runtime --example real_disk_file_system_tools
cargo run --manifest-path examples/coding-cli/Cargo.toml        # ratatui TUI coding agent (`ercode`)
cargo run --manifest-path examples/weekend-concierge-host/Cargo.toml
```

Built **with** the runtime:

- **[yolop](https://github.com/everruns/yolop)** — a minimal terminal coding
  agent on `everruns-runtime` (codex/claude-code in spirit); interactive TUI,
  real-filesystem tools, multi-provider LLMs, MCP (stdio + HTTP), resumable
  sessions, Zed/ACP integration.

  ```bash
  brew install everruns/tap/yolop   # or: cargo install yolop --locked
  ```

## References

- [API reference (docs.rs)](https://docs.rs/everruns-runtime)
- [Runtime guide](https://docs.everruns.com/features/runtime/)
- [Tutorial: build your first agent](https://docs.everruns.com/tutorials/building-agents-using-sdk/)
- [Embedding Everruns](https://docs.everruns.com/advanced/embedding-everruns/) — compose your own `PlatformDefinition`, server, and worker
- [Capabilities](https://docs.everruns.com/capabilities/) · [Harnesses](https://docs.everruns.com/features/harnesses/) · [Integrations](https://docs.everruns.com/integrations/)
- [Everruns documentation](https://docs.everruns.com) · [GitHub](https://github.com/everruns/everruns)
- In-repo specs: `specs/runtime.md`, `specs/embedding.md`, `specs/file-store.md`
- [`references/crates.md`](references/crates.md) — optional provider/integration/backend crate catalog
- [`references/glossary.md`](references/glossary.md) — domain language

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
