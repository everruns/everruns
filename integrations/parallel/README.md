# everruns-integrations-parallel

Parallel web search integration for Everruns agents.

This crate is part of the [Everruns](https://everruns.com) ecosystem. It
contributes Parallel's hosted MCP server so agents get provider-owned
`web_search` and `web_fetch` tools, free by default with an optional
Parallel API-key connection for authenticated usage.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_parallel::ParallelCapability;

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

## License

MIT. See the repository-level `LICENSE` file.
