# everruns-durable

> PostgreSQL-backed durable execution engine for Everruns.

`everruns-durable` is a generic workflow substrate: it runs typed activities and
workflows reliably on PostgreSQL, survives process restarts, and retries failed
work with backoff and circuit breaking. It contains no Everruns product API or
agent policy. [`everruns-scale`](https://docs.rs/everruns-scale) composes it into
the portable distributed Engine.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents.

## Quick Example

```rust
use everruns_durable::WorkflowRegistry;

// Register workflows in a registry, then run them with a `WorkflowExecutor`
// backed by a PostgreSQL event store (`PostgresWorkflowEventStore`).
let mut registry = WorkflowRegistry::new();
// registry.register("my-workflow", ...);
let _ = &mut registry;
```

## What It Provides

- A PostgreSQL-backed workflow executor with a workflow registry
- `Activity` / `ActivityContext` abstractions for retryable units of work
- Reliability primitives: retry policies and circuit breakers
- Persistence of workflow and activity state for crash-safe resumption

## Documentation

- [Durable execution](https://docs.everruns.com/explanation/durable-execution/)
- [API reference](https://docs.rs/everruns-durable)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
