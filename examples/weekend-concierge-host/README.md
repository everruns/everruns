# Weekend Concierge Host

A standalone embedder example for `everruns-runtime`.

This example lives in the root `examples/` folder on purpose: it behaves like a
small external host application, not like an internal crate-local demo.

## What It Shows

- a host-defined capability with proprietary local data
- a custom `PlatformDefinition`
- seeded workspace files available inside the runtime
- a deterministic in-process turn driven by `llmsim`
- how to inspect the resulting messages and emitted events

## Run

```bash
cargo run --manifest-path examples/weekend-concierge-host/Cargo.toml
```

## Test

```bash
cargo test --manifest-path examples/weekend-concierge-host/Cargo.toml
```

## What Happens

The example hosts a tiny "weekend concierge" app:

- the host provides a `lookup_neighborhood_spot` tool
- the harness gets a seeded `/workspace/welcome-note.md`
- the session gets a seeded `/workspace/weekend-brief.md`
- the in-process runtime handles the full `input -> reason -> act` loop

The console output prints:

- the seeded brief file
- the tools visible to the runtime
- the final response
- the message transcript
- the emitted event types, including `tool.completed`

The binary stays thin on purpose. The host wiring lives in `src/lib.rs`, so the
same deterministic flow is exercised by the automated test and the runnable
example.
