# Everruns Framework and advanced hosts

Start new Rust applications with the application-facing `everruns` Framework.
The engine runs in process without a database, server, or worker. Enable the
provider you intend to call and supply its credentials; `everruns` does not
re-export Tokio, so add it for `#[tokio::main]`:

```bash
cargo add everruns --features openai
cargo add tokio --features macros,rt-multi-thread
export OPENAI_API_KEY=...   # the person sets it in their shell, never in chat
```

A typed tool is an async function marked `#[everruns::tool]`: the macro derives
its JSON schema from the signature and doc comment, and `.tool(name())` adds it.

```rust
use everruns::{Agent, Engine, OpenAI};

/// Return the current Unix time in seconds.
#[everruns::tool]
async fn unix_time() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|error| error.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("Use unix_time when asked about time. Answer in one sentence.")
        .provider(OpenAI::from_env()?)
        .model("gpt-6.2-sol")
        .tool(unix_time())
        .build()?;
    let turn = Engine::new().create(agent).run("What time is it?").await?;
    println!("{}", turn.response);
    Ok(())
}
```

The API changes between releases. Check the version `cargo add` resolved and
read that version's examples on <https://docs.rs/everruns> (or the crate's
README in the Cargo registry) rather than relying on memory.

## Choose the dependency path

- Ordinary applications use `everruns` for agents, models/providers, tools,
  sessions, events, cancellation, workspaces, MCP, plugins, inspection, and
  opt-in local scheduling.
- Advanced system integrators combine `everruns`, `everruns-core` with the
  `host` feature, and `everruns-contracts`. Select concrete providers and
  integrations through facade features or `everruns::batteries`.

`everruns_core::host` owns the low-level host boundary. The execution kernel
lives in `everruns_core::engine`; do not add retired host, engine, or MCP crates.

## Common Framework tasks

- Configure vendors with typed values from `everruns`, such as
  `Anthropic::from_env()`, `Meta::new(key)` or `OpenRouter::from_env()`; enable
  the corresponding facade feature. Pass the value to `.provider(...)` or
  convert it into `Provider` for a low-level host. Ordinary applications do
  not need a direct `everruns-drivers` dependency.
- Select a chat protocol on the connection:
  `OpenAI::from_env()?.api(OpenAIApi::ChatCompletions)` or
  `Mistral::from_env()?.api(MistralApi::Responses)`. Import the types from
  `everruns`. Defaults remain OpenAI Responses and Mistral Chat Completions.
  Do not put API selection in shared `LlmCallConfig` or raw driver options.
  Providers with a single supported chat API need no selector. Use the
  [provider setup guide](https://everruns.com/docs/framework/models-and-providers/)
  for account, endpoint, AWS and host-owned OAuth configuration.

- Use `Model::simulated` or `Model::simulated_with_config` for deterministic,
  credential-free tests. Examples and real workflows use a live provider.
- Pair a plain Framework `Model` with `Provider`, or use a provider
  convenience, for real models.
- Add ordinary tools with `#[everruns::tool]` or `FunctionTool`; use the
  curated `everruns::capability` API for reusable capability packages.
- Observe `Session::events`, cancel through `RunOptions`, and inspect the next
  model context with `Session::inspect`.
- Seed files with `AgentBuilder::file` and `readonly_file`; attach one trusted
  workspace and configure `WorkspacePolicy` where needed.
- Configure scoped MCP through `McpServer` and plugins through
  `AgentBuilder::plugin`. Local-process MCP stays opt-in.
- Enable `everruns/local` for a real workspace plus local task/schedule state.
  The same profile also provides crash-durable canonical event history.

## Hosted agent applications

For an HTTP agent application with routes, tools, and approvals, see
[Everruns serve](https://everruns.com/docs/framework/serve/). Applications that
only call a hosted Everruns agent use an SDK; [sdk.md](sdk.md)
covers Python, TypeScript, and Rust clients.

## Advanced host boundary

Use low-level crates only when the application is itself an execution host and
must replace stores, filesystem factories, platform definitions, phase
adapters, durable scheduling lifecycle, or worker topology:

```bash
cargo add everruns everruns-contracts
cargo add everruns-core --features host
```

```rust
use everruns::Provider;
use everruns_core::host::{HostBackends, InProcessRuntimeBuilder};
use everruns_contracts::model_spec::ModelSpec;
# let _ = (HostBackends::in_memory, InProcessRuntimeBuilder::new);
```

Canonical events are the only maintained conversation write path. Advanced
hosts use `EventLog`, `EventHistory`, `EventSink`, and `EventReader`; legacy
mutable-history paths are not execution inputs.

## References

- [Everruns Framework](https://everruns.com/docs/framework/)
- [Custom backends](https://everruns.com/docs/framework/custom-backends/)
- [Framework API](https://docs.rs/everruns)
- [Host API](https://docs.rs/everruns-core/latest/everruns_core/host/)
- [Focused crate catalog](framework-crates.md)
- [Domain-language glossary](framework-glossary.md)
