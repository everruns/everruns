---
title: Serve on celld (experimental)
description: Run a serve app durably on celld, self-hosted Durable Objects, with everruns-serve-celld. The agent runs in a container, and a Durable Object snapshots, restores and replays its state.
sidebar:
  order: 3
---

> **Experimental.** Like [serve](/framework/serve/), `everruns-serve-celld` is a
> proof of concept with no compatibility promise.

[celld](https://github.com/denoland/celld) runs a Cloudflare Workers application,
Durable Objects included, on your own machines. Each Durable Object is a *cell*:
one node owns it at a time, its SQLite database is replicated to other nodes and
to an object-storage bucket, and another node takes it over when its owner dies.
`everruns-serve-celld` (imported as `serve_celld`) uses a cell to keep a
[serve](/framework/serve/) app durable.

## How it maps

| celld | serve |
|---|---|
| A cell, addressed by name | One serve app instance: name one per conversation or per user |
| The container the cell supervises | The serve binary, with its state on the container's ephemeral disk |
| The cell's SQLite | The latest snapshot of serve's state, and a journal of requests since |
| A new container (after a move, a node loss or an idle stop) | Restored from the snapshot, then the journal is replayed |
| The cell's alarm | Takes the next snapshot once the app is idle, and recovers after a node restart |

The agent never notices: the snapshot holds serve's session catalog, the engine's
event log and the workspace, so a restored container resumes every session.

## The container

Replace `serve::start` with `serve_celld::start` in `main`:

```rust
#[tokio::main]
async fn main() -> serve::Result {
    serve_celld::start(serve::App::builder().discover().build()).await
}
```

With no command the binary listens on port 8080 in `start` mode and adds three
routes for the cell: `GET /celld/state` (boot id and busy), `GET /celld/snapshot`
and `PUT /celld/restore`. serve boots on the first other request, so a restore can
happen first. Keep state at one path for every container of an image (the default,
`/tmp/serve-celld`, or `SERVE_DATA_DIR`), because the engine records the
workspace root.

`[sandbox] kind = "microvm"` gives the agent a shell and file tools in a workspace
inside the data directory, so snapshots carry it. The container is the boundary;
for code you did not write, give the container class a runtime with its own kernel
(`runtime: "runsc"` or `"kata"` in `wrangler.jsonc`).

## The cell

The cell is a small JavaScript Durable Object that extends `Container` from
`@cloudflare/containers`. Copy `worker/` from the
[celld example](https://github.com/everruns/everruns/tree/main/examples/serve/celld).
Requests reach a cell at `/cells/<name>/v1/...`, and the cell:

1. asks the container for its boot id, and restores and replays when the
   container is new;
2. journals a mutating request (a message, an approval, an answer, a cancel, an
   AG-UI run) before it forwards it;
3. snapshots once the app is idle and drops the journal the snapshot covers.

`POST /v1/sessions` is not journaled, since serve picks the session id. It is
answered only once a snapshot holds the new session. When another turn in the same
cell keeps the app busy for 30 seconds, the answer carries
`x-celld-durable: pending` and the snapshot follows when the turn ends.

## Guarantees and limits

- A request the cell answered survives the loss of the container or of the node.
  Durability is per turn: a replayed turn calls the model and runs its tools
  again, so tools with side effects need their own idempotency.
- Snapshots are stored whole in the cell and pass through the isolate heap
  (128 MB by default), which bounds the state one cell can hold.
- Schedules run inside the container, so they run only while it is up.
- The routes under `/celld/` are the cell's own; the Worker never forwards them.

## Try it

The example runs without a container engine too: its external cell reaches a serve
process by URL. Its end-to-end test kills the serve process mid-turn and then the
celld node itself, and checks that the conversation comes back whole. See the
[example README](https://github.com/everruns/everruns/tree/main/examples/serve/celld).

## The engine inside the cell

A second, experimental shape runs no container at all: the engine is compiled to
WebAssembly and runs inside the Durable Object, writing every event to the cell's
own SQLite. A turn advances one engine step at a time (one model call, or one batch
of tool calls), and each step commits in one transaction, so losing a node costs at
most the step that was running. It ships as an example, not a crate, and has one
built-in tool. See the
[engine cell example](https://github.com/everruns/everruns/tree/main/examples/celld-engine).
