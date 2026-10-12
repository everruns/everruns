---
name: everruns
description: Build Rust applications with the Everruns Framework or compose advanced execution hosts with everruns_core::host. Use for in-process agents, providers, tools, scoped MCP clients, and capability authoring. For managing hosted agents through the Everruns MCP endpoint, use everruns-mcp or everruns-agent-authoring.
license: MIT
compatibility: Rust 1.94+ (edition 2024)
metadata:
  category: agent-framework
  version: "3.1"
  homepage: https://everruns.com
  docs: https://docs.everruns.com/framework/
  api-reference: https://docs.rs/everruns
  repository: https://github.com/everruns/everruns
---

# Everruns Framework and advanced hosts

Start new Rust applications with the application-facing `everruns` Framework.
The engine runs in process without a database, server, or worker. Enable the
provider you intend to call and supply its credentials:

```bash
cargo add everruns --features openai
```

```rust
use everruns::{Agent, Engine, OpenAI};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Answer in one short sentence.")
    .provider(OpenAI::from_env()?)
    .model("gpt-6.1-sol")
    .build()?;
let turn = Engine::new().create(agent).run("Say hello.").await?;
println!("{}", turn.response);
# Ok(())
# }
```

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

- [Everruns Framework](https://docs.everruns.com/framework/)
- [Custom backends](https://docs.everruns.com/framework/custom-backends/)
- [Framework API](https://docs.rs/everruns)
- [Host API](https://docs.rs/everruns-core/latest/everruns_core/host/)
- [Focused crate catalog](references/crates.md)
- [Domain-language glossary](references/glossary.md)
