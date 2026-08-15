---
name: everruns
description: Route Rust applications to the Everruns Framework, or compose advanced execution hosts from everruns-host and focused crates. Use for in-process agents, custom host backends, providers, MCP, or capability authoring.
license: MIT
compatibility: Rust 1.94+ (edition 2024)
metadata:
  category: agent-framework
  version: "3.0"
  homepage: https://everruns.com
  docs: https://docs.everruns.com/framework/
  api-reference: https://docs.rs/everruns
  repository: https://github.com/everruns/everruns
---

# Everruns Framework and advanced hosts

Start new Rust applications with the application-facing `everruns` Framework.
Its default build runs offline with no database, server, worker, network, or
credentials:

```bash
cargo add everruns
```

```rust
use everruns::{Agent, InMemoryEngine, Model};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Answer in one short sentence.")
    .model(Model::simulated("Hello from Everruns."))
    .build()?;
let turn = InMemoryEngine::new().create(agent).run("Say hello.").await?;
assert_eq!(turn.response, "Hello from Everruns.");
# Ok(())
# }
```

## Choose the dependency path

- Ordinary applications use `everruns` for agents, models/providers, tools,
  sessions, events, cancellation, workspaces, MCP, plugins, inspection, and
  opt-in local scheduling.
- Advanced system integrators combine `everruns` with `everruns-host` and
  focused siblings such as `everruns-mcp`, provider crates, and integration
  crates. They import `HostBackends` and host phase contracts directly.

`everruns-host` is the only low-level host boundary; there is no separate
runtime crate.

## Common Framework tasks

- Use `Model::simulated` or `Model::simulated_with_config` for deterministic,
  credential-free tests.
- Pair `ModelSpec` and `Provider`, or use a provider convenience, for real
  models.
- Add ordinary tools with `#[everruns::tool]` or `FunctionTool`; use the
  curated `everruns::capability` API for reusable capability packages.
- Observe `Session::events`, cancel through `RunOptions`, and inspect the next
  model context with `Session::inspect`.
- Seed files with `AgentBuilder::file` and `readonly_file`; attach one trusted
  workspace and configure `WorkspacePolicy` where needed.
- Configure scoped MCP through `McpServer` and plugins through
  `AgentBuilder::plugin`. Local-process MCP stays opt-in.
- Enable `everruns/local` for a real workspace plus local task/schedule state.
  It does not imply durable conversation history.

## Advanced host boundary

Use low-level crates only when the application is itself an execution host and
must replace stores, filesystem factories, platform definitions, phase
adapters, durable scheduling lifecycle, or worker topology:

```bash
cargo add everruns everruns-host
```

```rust
use everruns::{ModelSpec, Provider};
use everruns_host::{HostBackends, InProcessRuntimeBuilder};
# let _ = (HostBackends::in_memory, InProcessRuntimeBuilder::new);
```

Canonical events are the only maintained conversation write path. Advanced
hosts use `EventLog`, `EventHistory`, `EventSink`, and `EventReader`; legacy
mutable-history paths are not execution inputs.

## References

- [Everruns Framework](https://docs.everruns.com/framework/)
- [Custom backends](https://docs.everruns.com/framework/custom-backends/)
- [Framework API](https://docs.rs/everruns)
- [Host API](https://docs.rs/everruns-host)
- [Focused crate catalog](references/crates.md)
- [Domain-language glossary](references/glossary.md)
