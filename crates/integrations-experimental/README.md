# everruns-integrations-experimental

> Opt-in integrations with an experimental support promise.

[Everruns](https://everruns.com) is an agentic runtime and control plane.

```toml
[dependencies]
everruns-integrations-experimental = { version = "0.43", default-features = false, features = ["deno"] }
```

```rust
#[cfg(feature = "deno")]
use everruns_integrations_experimental::deno as _;
```

## Features

This crate contains opt-in integrations whose APIs or vendor coverage have a different support promise from the maintained integrations in [`everruns-integrations`](https://crates.io/crates/everruns-integrations). Each module is selected with its own feature and implements contracts from `everruns-contracts`. These integrations may change.

| Feature | Module | Integration |
|---|---|---|
| `agentid` | `agentid` | AgentID sign-in through the agent's AgentMail inbox |
| `deno` | `deno` | Deno sandbox client |
| `sprites` | `sprites` | Sprites cloud sandboxes |

## Documentation

See the [integration guides](https://docs.everruns.com/integrations) and the [API reference](https://docs.rs/everruns-integrations-experimental).

## License

MIT. See the [repository license](https://github.com/everruns/everruns/blob/main/LICENSE).
