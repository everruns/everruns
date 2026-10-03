# everruns-mcp

> Deprecated compatibility shim for `everruns-core::mcp`.

[![Crates.io](https://img.shields.io/crates/v/everruns-mcp.svg)](https://crates.io/crates/everruns-mcp)
[![Documentation](https://docs.rs/everruns-mcp/badge.svg)](https://docs.rs/everruns-mcp)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

This final shim release forwards the existing API to `everruns-core`. The next
platform release removes this package. No capability is removed; enable the
`mcp` feature and import `everruns_core::mcp` instead.

Part of the [Everruns](https://everruns.com) ecosystem.

## Quick Example

```rust
use everruns_core::{DisabledEgressService, mcp::{McpClient, NoAuthProvider}};
use std::sync::Arc;
let client = McpClient::new(Arc::new(DisabledEgressService), Arc::new(NoAuthProvider));
let _ = client;
```

## Documentation

See the [public documentation](https://everruns.com/docs), the
[core API reference](https://docs.rs/everruns-core), and the
[compatibility API reference](https://docs.rs/everruns-mcp).

## License

[MIT](https://github.com/everruns/everruns/blob/main/LICENSE).
