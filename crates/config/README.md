# everruns-config

Small configuration helpers shared across the Everruns Rust crates.

This crate is part of the [Everruns](https://everruns.com) ecosystem. It keeps
environment-variable parsing consistent for services, workers, integrations,
and embedders without pulling in the larger runtime crates.

## Quick Example

```rust
use std::time::Duration;

use everruns_config::{env_bool, env_duration_secs, env_required, env_string};

let bind_addr = env_string("EVERRUNS_BIND_ADDR", "127.0.0.1:8080");
let allow_dev = env_bool("EVERRUNS_ALLOW_DEV", false);
let timeout = env_duration_secs("EVERRUNS_REQUEST_TIMEOUT_SECS", Duration::from_secs(30));
let database_url: String = env_required("DATABASE_URL")?;

println!("{bind_addr} {allow_dev} {timeout:?} {database_url}");
# Ok::<(), everruns_config::ConfigError>(())
```

## What It Provides

- Optional and required env-var readers
- Typed parsing with default values
- Duration helpers for seconds and milliseconds
- A shared `ConfigError` type for missing or invalid required values

## License

MIT. See the repository-level `LICENSE` file.
