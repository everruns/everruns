# everruns-config

> Shared configuration-loading helpers for Everruns crates.

[![Crates.io](https://img.shields.io/crates/v/everruns-config.svg)](https://crates.io/crates/everruns-config)
[![Documentation](https://docs.rs/everruns-config/badge.svg)](https://docs.rs/everruns-config)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-config` keeps environment-variable parsing consistent across the
Everruns crates — services, workers, integrations, and embedders — without
pulling in the larger runtime crates. It offers small, typed readers with
defaults and a shared error type for missing or invalid required values.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents.

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

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-config)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
