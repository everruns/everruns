# everruns-serve-celld

> Run ordinary serve applications on celld with per-session Engine-owned durable authority.

Part of the [Everruns](https://everruns.com) ecosystem.

Experimental native [serve](../serve) hosting over per-session celld Durable
Objects. Applications keep ordinary serve declarations. The adapter selects
`everruns::Engine` with `ObjectHosting` and exposes that same Engine at the
independently authenticated `/execute` activation endpoint.

## Quick Example

```rust
use serve::prelude::*;

#[agent]
fn assistant() -> Agent {
    Agent::builder().model("openai/gpt-6.1-sol").instructions("Answer concisely.").build()
}

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

See the [example](https://github.com/everruns/everruns/tree/main/examples/serve/celld) and
[Engine hosting architecture](https://everruns.com/docs/framework/architecture/).
The Engine profile migration and platform qualification are in progress; these
changes do not qualify production storage, unsupported host services or channel
outbox durability.

## What It Provides

- The ordinary serve `/v1` API and a separately authenticated Engine `/execute` endpoint.
- Stable per-session DO bindings and reconstructible catalog projections.
- Explicit Environment and sandbox adapters, independent of journal durability.

## Documentation

- [serve on celld](https://everruns.com/docs/framework/serve-celld/)
- [API reference](https://docs.rs/everruns-serve-celld)
- [Durable Object hosting](https://everruns.com/docs/framework/durable-objects/)

## License

MIT. See [LICENSE](https://github.com/everruns/everruns/blob/main/LICENSE).
