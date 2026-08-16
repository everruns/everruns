# Parallel Integration

This crate hosts two independent Parallel capabilities. They share only the vendor:

- `parallel_search`, **free**, connection-backed. Contributes Parallel's hosted MCP
  server for web search/fetch. Experimental (dev-only).
- `parallel`, **paid**, payment-backed. Implements in-process search/extract/task tools
  that spend real money through the core `PaymentAuthority`. Gated by the
  deployment-controlled `machine_payments` feature flag (off by default everywhere).

The two have opposite trust models and share no code. Core owns the payment trust
boundary (`PaymentAuthority`, payment DTOs, `ToolContext`); this crate owns only the
thin vendor adapters.

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

## `parallel` capability (paid, machine payments)

The `parallel` capability contributes in-process tools that route paid calls through the
core `PaymentAuthority`. See [`knowledge/security/machine-payments.md`](../../knowledge/security/machine-payments.md)
for the trust boundary and rails.

- `parallel_search`, paid web search. Up to `$0.01` per call.
- `parallel_extract`, structured extraction. Up to `$0.01` per URL, minimum `$0.01`.
- `parallel_task`, deep async task. Up to `$0.10` (`pro`) / `$0.30` (`ultra`).
- `parallel_task_status`, free polling of a task run; no payment.

The paid tools never sign or submit payments themselves: they build a typed
`MachinePaymentRequest` and call `ToolContext.payment_authority`. With no authority wired
(the default), they fail closed. The capability is registered only when the
`machine_payments` feature flag is on (`FEATURE_MACHINE_PAYMENTS=true`), off by
default on every grade including dev because spend is irreversible.

## Tests

- `tests/plugin_registration.rs` verifies inventory registration for both capabilities and
  the connection provider, and that the paid `parallel` capability is flag-gated
  (registered only when `FEATURE_MACHINE_PAYMENTS` is set).
- `tests/live_api_test.rs` runs live no-secret MCP smoke tests against the free endpoint with `--features integration`.
- Unit tests in `src/payments.rs` cover paid-tool metadata and fail-closed behavior when no
  payment authority is configured.

## Security

The free MCP path uses the shared MCP URL validation path before discovery and execution. API keys are user-scoped connections, encrypted by the existing connection storage, and only sent as bearer auth when capability config opts into authenticated mode or the OAuth-compatible endpoint.

The paid capability inherits the machine-payments mitigations (`TM-AGENT-022`, `TM-CRYPTO-008`): no generic paid-HTTP tool, capability/host allowlists and per-request caps enforced server-side by `PaymentAuthority`, wallet custody and signing kept in the control plane, and the `machine_payments` flag gating registration so a money-spending capability is never offered by default.
