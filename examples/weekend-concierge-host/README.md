# Weekend Concierge Host

A standalone application example for the `everruns` Framework API.

This example lives in the root `examples/` folder on purpose: it behaves like a
small external host application, not like an internal crate-local demo.

## What It Shows

- an application-defined function tool with proprietary local data
- seeded workspace files available inside a Framework session
- a deterministic in-process turn driven by `llmsim`
- how to inspect the resulting context and observe session events

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
- the agent gets seeded `/workspace/welcome-note.md` and `/workspace/weekend-brief.md` files
- the Framework session handles the full `input -> reason -> act` loop

The console output prints:

- the seeded brief file
- the tools visible to the runtime
- the final response
- the message transcript
- the emitted event types, including `tool.completed`

The binary stays thin on purpose. The host wiring lives in `src/lib.rs`, so the
same deterministic flow is exercised by the automated test and the runnable
example.
