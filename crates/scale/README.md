# everruns-scale

> Portable PostgreSQL-backed Engine implementation for Everruns.

`everruns-scale` runs explicitly portable agent definitions across process and
worker boundaries. Definitions persist only stable registration paths;
providers, tools, capabilities, hooks, and workspace implementations are
resolved from an application registry at worker startup. Arbitrary
`everruns::Agent` values are rejected because they may contain closures,
drivers, event sinks, or other process-local code.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents.

## Quick Example

```rust
use everruns_scale::{PortableAgent, RegistrationPath};

let definition = PortableAgent::builder(
    "Answer concisely.",
    "gpt-5-mini",
    RegistrationPath::new("providers/openai/default")?,
    RegistrationPath::new("workspaces/project/v1")?,
)
.tool(RegistrationPath::new("tools/weather/v1")?)
.build()?;

assert_eq!(definition.version, 1);
# Ok::<(), everruns_scale::PortableAgentError>(())
```

Create a `Registry`, register every referenced implementation during process
bootstrap, initialize `ScaleEngine::new` with a PostgreSQL pool, then call
`ScaleEngine::submit`. Schema migrations are versioned and can initialize a
blank database. The crate does not depend on `everruns-server` or
`everruns-worker`.

## What It Provides

- A public `everruns::Engine` implementation with a fail-closed portability boundary
- Versioned portable definitions and stable component registration paths
- Generic durable run control shared by embedded, direct PostgreSQL, and remote workers
- Blank-database PostgreSQL schema setup, restart recovery, cancellation, and steering

## Documentation

- [Scalable engines](https://docs.everruns.com/framework/scalable-engines/)
- [API reference](https://docs.rs/everruns-scale)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
