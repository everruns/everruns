# everruns-serve-celld

Experimental native [serve](../serve) hosting over per-session celld Durable
Objects. Applications keep ordinary serve declarations. The adapter selects
`everruns::Engine` with `ObjectHosting` and exposes that same Engine at the
independently authenticated `/execute` activation endpoint.

```rust
#[tokio::main]
async fn main() -> serve::Result {
    serve_celld::start(serve::App::builder().discover().build()).await
}
```

With no command or `celld`, startup requires `AUTHORITY_URL`, `TENANT_ID`,
`AUTHORITY_TOKEN` and a distinct `EXECUTOR_TOKEN`. Configure `SERVE_API_KEYS` for
the user API. Other commands go to ordinary serve. Missing configuration fails
before listening. `SERVE_DATA_DIR` holds only the local catalog/cache; authority
binding metadata can reconstruct the session catalog after container loss.

The [Worker](../../examples/serve/celld/worker) has one session authority per
tenant/session and a separate replaceable executor container. Creation commits
its binding before acknowledgement, including while another session runs.
Admission, receipts, canonical events, epochs and alarms belong to the session
DO. Containers never restore a canonical snapshot or replay accepted HTTP
requests. Old `/cells/...` and `/celld/snapshot`/`restore` protocols are removed.

`SERVE_WORKSPACE_DIR` supplies separate persistent workspace directories by session.
Its lifetime is independent of the journal. For OS commands, configure an explicit
`Options::microvm` sandbox adapter and, when needed, `Options::environment` host
services. The adapter must isolate commands from the Engine process, other sessions
and privileged authority credentials. A directory or the executor container alone
does not provide that isolation; the adapter never enables an uncontained shell.

See the [example](../../examples/serve/celld) and
[Engine hosting architecture](../../knowledge/framework/engine-durable-sessions.md).
The Engine profile migration and platform qualification are in progress; these
changes do not qualify production storage, unsupported host services or channel
outbox durability.
