---
title: Event triggers
description: Route authenticated events through shared filters, changes-only checks, durable replay protection and recoverable Engine admission.
---

Enable `triggers` to use the Server's filtering, change detection, template rendering and GitHub normalization without its management APIs. An application owns HTTP authentication, subscriptions and scheduling; `Dispatcher` owns the event-to-Engine pipeline.

```rust
# fn configure(engine: &everruns::Engine, build: everruns::hosting::AgentBuild) -> Result<(), Box<dyn std::error::Error>> {
use everruns::triggers::{Dispatcher, SqliteStore, Trigger, Filter, Routing};

let dispatcher = Dispatcher::new(engine, SqliteStore::open("private-triggers.sqlite")?, Trigger {
    key: "deployment-watcher/v1".into(),
    build,
    message: "Deployment {{payload.version}} completed for {{payload.service}}".into(),
    filter: Filter::default(),
    routing: Routing::PerSubject,
})?;
# Ok(())
# }
```

Register the exact `AgentBuild` with the Engine before dispatch. Change the trigger key when changing its configuration; accidental reuse is rejected. Routing can create one session per event, per subject, or one shared session. Missing subjects under `PerSubject` use an event-specific session.

Filters combine dotted paths and accepted values. Numeric/boolean scalars match their string representations using Server semantics. `ChangesOnly` remembers the last value per subject: the first event establishes the baseline, repeats are filtered, and a change passes. Events missing the selected value do not erase it. The latest 500 subjects are retained.

## Recovery and replay protection

Before calling the Engine, a transaction freezes the rendered input, session and input IDs in a durable outbox. Concurrent retries and recovery use `Session::send_with_id` to admit the same input once. A delivery reports `Dispatched` after admission, not after the agent finishes; use its session/input IDs to inspect or wait for the admission.

Call `recover()` at application startup and from your scheduling loop. It retries unadmitted deliveries. Engine hosting and the outbox must both survive process loss. At most 200 unadmitted deliveries are accepted per trigger; recent payload history is bounded. Permanent receipt tombstones retain replay protection even after delivered/filtered payload history is pruned. Retire the database explicitly when that retention is no longer required.

`StateStore` lets applications replace SQLite. Its compare-and-swap must atomically update state and insert any new receipt, and receipt tombstones must outlive history pruning. Store state outside the agent workspace, with application-level access restrictions.

## Sources

`Event` accepts already authenticated application events. Never construct it directly from a public webhook without authentication.

`github(secret, GitHubRequest, own_bot)` verifies `X-Hub-Signature-256` over the exact body before parsing, suppresses the selected bot's own events, and shares GitHub/CI context normalization with Server. Restrict subscribed event types and repositories in the ingress/filter configuration. GitHub signatures do not include timestamps; durable delivery IDs provide replay protection.

`mcp(secret, McpRequest)` uses the shared Standard Webhooks verifier, enforces the timestamp window and expected live subscription, then builds MCP template context. The application owns subscribe/renew/unsubscribe calls and passes only the active subscription identity. Oversized bodies and invalid signatures are rejected before dispatch.

The [complete trigger example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/service_triggers.rs) uses a real model and persistent local storage. Run it twice with the same event ID to see durable duplicate suppression:

```bash
cargo run -p everruns --features openai,local,triggers --example service_triggers -- deploy-123
```
