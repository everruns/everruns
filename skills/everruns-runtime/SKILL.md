---
name: everruns-runtime
description: Route Rust applications to the Everruns Framework, migrate existing everruns-runtime 0.17.x code, or compose advanced hosts from everruns-host and focused crates. Use for in-process agents, runtime migration, custom host backends, providers, MCP, or compatibility maintenance.
license: MIT
compatibility: Rust 1.94+ (edition 2024)
metadata:
  category: agent-framework
  version: "2.0"
  homepage: https://everruns.com
  docs: https://docs.everruns.com/framework/runtime-compatibility/
  api-reference: https://docs.rs/everruns
  repository: https://github.com/everruns/everruns
---

# Everruns Framework and runtime compatibility

Start new Rust applications with the application-facing `everruns` Framework.
Its default build runs offline with no database, server, worker, network, or
credentials:

```bash
cargo add everruns
```

```rust
use everruns::{Agent, Model};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Answer in one short sentence.")
    .model(Model::simulated("Hello from Everruns."))
    .build()?;
let turn = agent.session().run("Say hello.").await?;
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
- Existing `everruns-runtime` 0.17.x applications remain supported while they
  migrate. The crate is compatibility-only and will be removed in 0.18.

The authoritative [runtime migration guide](https://docs.everruns.com/framework/runtime-compatibility/)
contains the exact old-to-new mapping. Do not reproduce that matrix here.

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

## Existing 0.17 compatibility work

When maintaining an existing runtime application, preserve behavior first,
then migrate one concern at a time using the public guide. Compatibility paths
re-export the same canonical host implementation, so old and new applications
must converge on the same behavior. Keep compatibility builds green under
`-D warnings`; do not add blanket compiler deprecation attributes.

## References

- [Everruns Framework](https://docs.everruns.com/framework/)
- [Runtime migration](https://docs.everruns.com/framework/runtime-compatibility/)
- [Custom backends](https://docs.everruns.com/framework/custom-backends/)
- [Framework API](https://docs.rs/everruns)
- [Host API](https://docs.rs/everruns-host)
- [Focused crate catalog](references/crates.md)
- [Legacy-to-Framework glossary](references/glossary.md)
