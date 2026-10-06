# everruns-integrations

This integration is shipped as the `parallel` module of `everruns-integrations` Enable it with `features = ["parallel"]` in Cargo.

> Parallel web search and fetch for Everruns agents.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations` contributes [Parallel](https://parallel.ai)'s
hosted MCP server so agents get provider-owned `web_search` and `web_fetch`
tools, free by default, with an optional Parallel API-key connection for
authenticated usage.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with
[`everruns-contracts`](https://crates.io/crates/everruns-contracts) through the Everruns
integration plugin system.

## Quick Example

```rust
use everruns_contracts::runtime::capabilities::Capability;
use everruns_integrations::parallel::ParallelCapability;

let capability = ParallelCapability;

assert_eq!(capability.id(), "parallel_search");
```

## What It Provides

- `parallel_search` capability contributing the hosted Parallel MCP server
  (`mcp_parallel__web_search`, `mcp_parallel__web_fetch`)
- Optional user-scoped Parallel API-key connection with bearer auth and an
  OAuth-compatible endpoint mode
- A separate paid `parallel` capability (search/extract/task tools) that routes
  spend through the core `PaymentAuthority`; gated behind the
  `machine_payments` feature flag and off by default
- Inventory-based Everruns integration registration

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations)
- [Parallel integration](https://docs.everruns.com/integrations/parallel/)
- [Give an agent web access](https://docs.everruns.com/how-to/give-an-agent-web-access/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
