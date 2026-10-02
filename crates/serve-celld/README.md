# everruns-serve-celld

> Run a [serve](https://crates.io/crates/everruns-serve) app durably on celld. Imported as `serve_celld`.

[![Crates.io](https://img.shields.io/crates/v/everruns-serve-celld.svg)](https://crates.io/crates/everruns-serve-celld)
[![Documentation](https://docs.rs/everruns-serve-celld/badge.svg)](https://docs.rs/everruns-serve-celld)
[![License](https://img.shields.io/crates/l/everruns-serve-celld.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

**Experimental.** A hosting target for serve in the [Everruns](https://everruns.com)
ecosystem. Like serve, it is a proof of concept with no compatibility promise.

[celld](https://github.com/denoland/celld) runs Cloudflare Workers and Durable
Objects on your own machines. A Durable Object (a *cell*) has one owner node at a
time and a SQLite database that celld replicates and fails over; a container the
cell supervises loses its disk when the cell moves. So the agent runs in the
container, and the cell keeps a snapshot of serve's state plus a journal of the
requests since. A fresh container is restored and the journal replayed, so a turn
a crash interrupted runs again.

This crate is the container half. The cell is a small JavaScript Durable Object in
[`examples/serve/celld/worker`](https://github.com/everruns/everruns/tree/main/examples/serve/celld/worker).

## What It Provides

| Route | What it does |
|---|---|
| `GET /celld/state` | `{"boot_id", "booted", "busy"}`. A new `boot_id` means a fresh container. Never boots the app |
| `GET /celld/snapshot` | serve's data directory as a tar, SQLite databases copied through the online backup API. `409` while a turn runs |
| `PUT /celld/restore` | Unpack a snapshot into the data directory. `409` once the app has booted |
| `/health`, `/v1/...` | serve's own wire API, booted on the first request so a restore can come first |

## Quick Example

```rust
use serve::prelude::*;

#[agent]
fn assistant() -> Agent {
    Agent::builder()
        .model("anthropic/claude-sonnet-5")
        .instructions("Be brief.")
        .build()
}

#[tokio::main]
async fn main() -> serve::Result {
    serve_celld::start(App::builder().discover().build()).await
}
```

With no command the binary serves the contract on port 8080 in serve's `start`
mode; `celld --dev` runs it offline. Other commands are serve's.

## Guarantees

- Turn-level durability: a request the cell acknowledged survives the loss of the
  container or the node. A replayed turn calls the model and its tools again.
- `POST /v1/sessions` is acknowledged only after a snapshot holds the session.
- State lives in `SERVE_DATA_DIR` (default `/tmp/serve-celld`). The engine records
  the workspace root, so every container of an image must use the same path.

## Documentation

- [Serve on celld](https://docs.everruns.com/framework/serve-celld/)
- [API reference](https://docs.rs/everruns-serve-celld)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
