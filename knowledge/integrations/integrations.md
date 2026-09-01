---
type: Specification
title: "Integrations"
description: "Integration specs index."
tags:
  - everruns
  - integrations
---
# Integrations

Index of external service integrations. Each spec lives alongside the code that implements it.

## Integration Parity Requirements

Every sandbox/execution integration crate must ship with the following artifacts to be considered production-ready. Use Daytona as the reference implementation.

| Requirement | Description |
|---|---|
| **SPEC.md** | Co-located specification: architecture, tool surface, state management, security review. |
| **Connection provider** | User connection (OAuth or API-key form) with validation and fallback env vars for operator/test use. |
| **Unit tests** | Plugin registration, parameter validation, state serialization, tool schemas. |
| **Integration tests** | `tests/tool_integration.rs`, tool `execute_with_context` flows against mocked storage/connections (wiremock for HTTP APIs, mock storage for websocket APIs). |
| **Live API tests** | `tests/live_api_test.rs`, feature-gated (`<name>-live-tests`), credentials read from required environment variables. In CI, those env vars are injected via Doppler. Missing-credential behavior is **fail-closed**: when the feature flag is on but the required env var is missing or empty, the test must `panic!` (not `eprintln!` + `return`). CI live jobs must never silently pass. Reference implementations: `integrations/brave-search/tests/smoke_real_api.rs`, `integrations/daytona/tests/live_api_test.rs`. |
| **CI: unit tests** | Crate listed in the `unit-test` job: `cargo test -p everruns-integrations-<name>`. |
| **CI: change detection** | Path filter in `changes` job: `<name>: integrations/<name>/**`. |
| **CI: live-test job** | Live API or real-API coverage must follow the repo trigger policy: cheap/path-local API smoke may run on `pull_request`; costly or stateful live jobs stay on `push` to `main`; path-filtered workflows must also be covered by the weekly/on-demand backstop in `.github/workflows/integration-live-sweep.yml`. New integrations: dedicated `.github/workflows/<name>-integration.yml` workflow for change-scoped runs plus inclusion in the full sweep. Legacy integrations (Daytona, Browserless, etc.) may still use a job in `ci.yml` for change-scoped runs. |
| **User docs** | `docs/integrations/<name>.md`, quick start, tool table, lifecycle, security. |
| **UI test case** | `test_cases/ui/<name>_connection/TC001_*.md`, manual test for connection + sandbox lifecycle. |
| **Seed agent** | Entry in `crates/server/src/seed.rs` with capabilities wired. |
| **Threat model** | Section in `knowledge/security/threat-model.md` covering integration-specific threats. |

New integrations should check off every row before merging. Existing integrations that are missing items should be brought up to parity incrementally.

### Auth shape: connection-backed vs payment-backed

The parity table above assumes a **connection-backed** integration: per-user OAuth/API-key
credentials via a connection provider, validated with live-API tests. A **payment-backed**
integration instead spends through the core `PaymentAuthority` (see
[`knowledge/security/machine-payments.md`](../security/machine-payments.md)) under a server-side wallet and policy,
and has no per-user connection. For those, substitute the auth-shaped rows:

| Connection-backed row | Payment-backed equivalent |
|---|---|
| Connection provider (OAuth/API-key form) | Declared `machine_payments` feature-flag gating + wallet/policy requirements |
| Live API tests (real credentials) | Fail-closed tests (no authority → refuse) + `PaymentAuthority`-mock flows |
| Threat model: integration-specific | Threat model: spend / prompt-injection (`TM-AGENT-022`, `TM-CRYPTO-008`) |

A single vendor crate may host both shapes (e.g. `integrations/parallel` ships the free
connection-backed `parallel_search` and the paid payment-backed `parallel`).

## CI Trigger Policy

Integration coverage is intentionally split by cost and blast radius:

- **`pull_request`**: keep PR feedback fast. Only cheap/path-local real-API coverage belongs here (for example DuckDuckGo and Brave Search). Stateful or higher-cost live sandbox jobs do not run on every PR.
- **`push` to `main`**: run change-scoped live jobs for integrations whose real coverage needs secrets, remote sandboxes, browsers, or Docker-in-Docker. These jobs stay path-filtered so unrelated PRs do not pay that cost before merge.
- **Weekly + on demand**: `.github/workflows/integration-live-sweep.yml` runs the full live/real-API matrix on a schedule and via `workflow_dispatch`. This is the backstop for regressions introduced through shared crates, harness code, workflow glue, or dependency bumps that per-integration path filters would miss.

The policy trades PR load for a bounded amount of post-merge and scheduled coverage. If a new integration needs a different trigger shape, document the cost/risk reason in its co-located spec.

## Integration Crates (`integrations/`)

Auto-registered via `inventory` plugin system. Each crate has a `SPEC.md`.

| Integration | Spec | Summary |
|---|---|---|
| Brave Search | [`integrations/brave-search/SPEC.md`](../../integrations/brave-search/SPEC.md) | Web search via Brave Search API. Experimental (Dev only). |
| Parallel | [`integrations/parallel/SPEC.md`](../../integrations/parallel/SPEC.md) | Two capabilities: free `parallel_search` (web search/fetch via Parallel MCP, Dev only) and paid `parallel` (machine-payment search/extract/task, gated by `FEATURE_MACHINE_PAYMENTS`). |
| DuckDuckGo | [`integrations/duckduckgo/SPEC.md`](../../integrations/duckduckgo/SPEC.md) | Instant answers via DuckDuckGo API. Experimental (Dev only). |
| GitHub | [`integrations/github/SPEC.md`](../../integrations/github/SPEC.md) | GitHub Scout blueprint capability for read-only repository exploration. |
| Browserless | [`integrations/browserless/SPEC.md`](../../integrations/browserless/SPEC.md) | Cloud browser automation, screenshots, DOM, scraping, multi-step interactions. REST and CDP modes. |
| E2B | [`integrations/e2b/SPEC.md`](../../integrations/e2b/SPEC.md) | Cloud sandbox environments via E2B management API + envd runtime endpoints. BYO API key via connection provider, multiple sandboxes per session. |
| Daytona | [`integrations/daytona/SPEC.md`](../../integrations/daytona/SPEC.md) | Cloud sandbox environments via Daytona REST API. Multiple sandboxes per session. |
| Deno | [`integrations/deno/SPEC.md`](../../integrations/deno/SPEC.md) | Cloud sandbox environments via Deno websocket sandbox API. Multiple sandboxes per session. **Unsupported — untested.** Deno sandboxes require a paid plan the project does not hold, so there is no live coverage and the integration is not exercised end to end; only mock-backed unit tests run. Not listed in the public docs. See EVE-946. |
| Sprites | [`integrations/sprites/SPEC.md`](../../integrations/sprites/SPEC.md) | Persistent Firecracker microVMs via Sprites (Fly.io). Persistent filesystem, checkpoints, HTTP services. |
| Cursor | [`integrations/cursor/SPEC.md`](../../integrations/cursor/SPEC.md) | Cursor Cloud Agents API for launching and managing asynchronous coding agents on GitHub repositories. |
| Docker | `integrations/docker/` | Container-based agent execution. Experimental (Dev only). No spec yet. |

> **ARD is not an integration.** Agentic Resource Discovery is a platform-level
> discovery protocol (a sibling of MCP and A2A), so it lives in `crates/ard`
> (`everruns-ard`), not under `integrations/`. See
> [`crates/ard/SPEC.md`](../../crates/ard/SPEC.md).

## Messaging Integrations (`crates/server/`)

Platform adapters connecting agents to messaging channels. Uses the channel abstraction layer defined in [`knowledge/integrations/messaging-integrations.md`](messaging-integrations.md) (`InboundChannelEvent`, `ChannelDeliveryAdapter`, `SessionRoutingStrategy`, `ThreadContext`).

| Integration | Spec | Summary |
|---|---|---|
| Slack Bot | [Slack Integration](slack-integration.md) | Deploy agents as Slack bots. Uses `InboundChannelEvent` for parsing, `build_session_routing_tag()` for routing, `SlackDeliveryAdapter` implementing `ChannelDeliveryAdapter`. |

## Server Integrations (`crates/server/`)

Embedded in the server crate.

| Integration | Spec | Summary |
|---|---|---|
| User Connections | [User Connections](user-connections.md) | OAuth/API-key connections to GitHub, GitLab, Bitbucket, Daytona for repo and sandbox access. |
| Valkey Cache | [Caching and Rate Limiting](../operations/caching-and-rate-limiting.md) | Distributed rate limiting via Valkey; in-process caching via `moka`. |

## Observability (`knowledge/operations/`)

| Integration | Spec | Summary |
|---|---|---|
| Braintrust + OpenTelemetry | [`knowledge/operations/observability.md`](../operations/observability.md) | Observability providers, OTel Gen-AI tracing and Braintrust event forwarding. |
