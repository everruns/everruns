---
title: Serve on celld (experimental)
description: "Run Framework Engine sessions on celld with per-session Durable Object authority and replaceable native execution."
sidebar:
  order: 3
---

`everruns-serve-celld` is experimental. It hosts ordinary serve agents through
`everruns::Engine`; the hosting plugin selects celld Durable Objects for session
authority. The profile migration and platform qualification are in progress.

Each tenant/session has its own authority object. That object commits creation,
admission receipts, canonical history and execution epochs, and retains alarms
for unfinished work. Native containers execute the registered Framework build.
They can be replaced without restoring canonical snapshots or replaying accepted
HTTP requests.

```rust
#[tokio::main]
async fn main() -> serve::Result {
    serve_celld::start(serve::App::builder().discover().build()).await
}
```

Configure `AUTHORITY_URL`, `TENANT_ID`, `AUTHORITY_TOKEN` and a distinct
`EXECUTOR_TOKEN` on the native host. Configure user API authentication separately
with `SERVE_API_KEYS`. Startup refuses missing authority configuration.

The Worker exposes the ordinary `/v1` routes and addresses authority objects by
authenticated tenant and typed session identity. Keep creation and message IDs
when retrying uncertain replies. Creation acknowledges the committed binding;
a successful send acknowledges durable admission. Turn assignment and completion
are observed separately. Disconnecting an event reader does not cancel work.

`SERVE_WORKSPACE_DIR` reopens separate workspace directories by session. OS tools
also need an explicit sandbox adapter through `serve_celld::Options::microvm`;
remote workspace/compute services can be supplied through `Options::environment`.
Commands must be isolated from the Engine process and its authority credentials.
The adapter never enables an uncontained shell. Session journal durability does
not preserve ephemeral container files. The local
catalog/cache can be reconstructed from authority binding metadata. Channel inbox,
outbox and scheduling guarantees require separately supported host services.

The former `/cells/<name>` routing and `/celld/snapshot`/`restore` protocol are
removed. Existing deployments using that experimental protocol need an explicit
migration; their snapshots are not the new authority format.

See the [runnable example](https://github.com/everruns/everruns/tree/main/examples/serve/celld)
for setup and deployment. The [isolate example](https://github.com/everruns/everruns/tree/main/examples/celld-engine)
uses the same Framework Agent and Engine as Cloudflare, without a container or a
celld-specific agent loop. Workerd tests and a local replicated fleet establish
only their tested failure modes; production bucket consistency, alarms, build
retention and workspace persistence remain deployment responsibilities.
