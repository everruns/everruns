# Parallel Integration

This crate hosts the `parallel_search` capability: Parallel's hosted MCP server for
web search and fetch, free by default with an optional API-key connection.

The paid `parallel` capability (machine payments) was removed in 2026-11 together
with the never-enabled `machine_payments` feature.

## `parallel_search` capability (free MCP)

The `parallel_search` capability contributes one scoped remote MCP server named `Parallel`.
Tools are discovered from the contributed MCP server so agents see a single MCP-backed tool surface.

Default mode uses `https://search.parallel.ai/mcp` without authentication. Optional config:

- `auth: "connection"` sends the user-scoped `parallel` connection token as a bearer token.
- `endpoint: "oauth"` switches the server URL to `https://search.parallel.ai/mcp-oauth` and requires the same user-scoped connection token.

## Connection

The `parallel` connection provider stores an optional Parallel API key. Validation calls `tools/list` on the free MCP endpoint with bearer auth; `401`/`403` means the key is invalid.

## Tools

Parallel currently exposes:

- `web_search` for ranked web search with dense excerpts.
- `web_fetch` for extracting focused markdown from known URLs.

The Everruns-visible tool names use the normal MCP prefix: `mcp_parallel__web_search` and `mcp_parallel__web_fetch`.

## Tests

- `tests/parallel_plugin_registration.rs` verifies the crate publishes only the free
  `parallel_search` capability.
- `tests/live_api_test.rs` runs live no-secret MCP smoke tests against the free endpoint with `--features integration`.

## Security

The free MCP path uses the shared MCP URL validation path before discovery and execution. API keys are user-scoped connections, encrypted by the existing connection storage, and only sent as bearer auth when capability config opts into authenticated mode or the OAuth-compatible endpoint.
