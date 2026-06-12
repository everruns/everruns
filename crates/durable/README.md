# everruns-durable

> PostgreSQL-backed durable execution engine for Everruns.

`everruns-durable` is the workflow orchestration engine that makes Everruns
agents *durable*: it runs activities and workflows reliably on top of PostgreSQL,
surviving process restarts and retrying failed work with backoff and circuit
breaking. It is an internal building block of the Everruns workspace — the server
and worker use it to keep long-running agent sessions progressing.

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
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
