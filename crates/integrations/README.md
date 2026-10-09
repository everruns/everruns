# everruns-integrations

> Vendor integrations for Everruns agents, selected independently by Cargo feature.

[Everruns](https://everruns.com) is an agentic runtime and control plane.

Vendor integrations for Everruns, grouped in one published crate. Enable only the vendor features your host needs. Every module implements `everruns-contracts`; the crate has no production dependency on `everruns-core`.

```toml
[dependencies]
everruns-integrations = { version = "0.42", default-features = false, features = ["filesystem", "bashkit"] }
```

```rust
#[cfg(feature = "filesystem")]
use everruns_integrations::filesystem::FileSystemCapability;

#[cfg(feature = "filesystem")]
let _ = std::any::type_name::<FileSystemCapability>();
```

## Features

| Feature | Module | Provides |
|---|---|---|
| `bashkit` | `bashkit` | Bashkit shell and hook execution |
| `brave-search` | `brave_search` | Brave web search |
| `browserless` | `browserless` | Browserless cloud browser |
| `cursor` | `cursor` | Cursor cloud agent |
| `daytona` | `daytona` | Daytona sandboxes |
| `docker` | `docker` | Docker container sandbox |
| `duckduckgo` | `duckduckgo` | DuckDuckGo search |
| `e2b` | `e2b` | E2B sandboxes and computer use |
| `filesystem` | `filesystem` | Session filesystem tools |
| `github` | `github` | GitHub repository tools |
| `lua` | `lua` | Lua code mode |
| `openai-decisions` | `openai_decisions` | OpenAI decision services |
| `openai-image` | `openai_image` | OpenAI image generation |
| `openrouter` | `openrouter` | OpenRouter provider integration |
| `parallel` | `parallel` | Parallel search |
| `typesafe` | `typesafe` | TypeSafe decisions |
| `web-fetch` | `web_fetch` | Authenticated web fetch |
| `webhook-channel` | `webhook_channel` | Generic JSON webhook channel driver |
| `modal` | `modal` | Modal sandboxes |

Each module exports capability and connector types for direct registration, plus plugin descriptors for the hosted catalog. Fake/demo capabilities remain in `everruns-test-support`; they are not registered by this crate.

Use `everruns-integrations` for the maintained integrations. Deno and Sprites, with their separate support promise, live in [`everruns-integrations-experimental`](https://docs.rs/everruns-integrations-experimental).

## Documentation

See the [integration guides](https://docs.everruns.com/integrations) and the [API reference](https://docs.rs/everruns-integrations). Each module README documents its vendor configuration.

## License

MIT. See the [repository license](https://github.com/everruns/everruns/blob/main/LICENSE).
