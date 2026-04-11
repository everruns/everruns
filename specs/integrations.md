# Integrations

Index of external service integrations. Each spec lives alongside the code that implements it.

## Integration Parity Requirements

Every sandbox/execution integration crate must ship with the following artifacts to be considered production-ready. Use Daytona as the reference implementation.

| Requirement | Description |
|---|---|
| **SPEC.md** | Co-located specification: architecture, tool surface, state management, security review. |
| **Connection provider** | User connection (OAuth or API-key form) with validation and fallback env vars for operator/test use. |
| **Unit tests** | Plugin registration, parameter validation, state serialization, tool schemas. |
| **Integration tests** | `tests/tool_integration.rs` — tool `execute_with_context` flows against mocked storage/connections (wiremock for HTTP APIs, mock storage for websocket APIs). |
| **Live API tests** | `tests/live_api_test.rs` — feature-gated (`<name>-live-tests`), optional Doppler credentials. |
| **CI: unit tests** | Crate listed in the `unit-test` job: `cargo test -p everruns-integrations-<name>`. |
| **CI: change detection** | Path filter in `changes` job: `<name>: integrations/<name>/**`. |
| **CI: live-test job** | Live API test job conditional on `push` event + Doppler token. New integrations: dedicated `.github/workflows/<name>-integration.yml` workflow, path-filtered to `integrations/<name>/**`. Legacy integrations (Daytona, Browserless, etc.) may still use a job in `ci.yml` gated on change detection. |
| **User docs** | `docs/integrations/<name>.md` — quick start, tool table, lifecycle, security. |
| **UI test case** | `test_cases/ui/<name>_connection/TC001_*.md` — manual test for connection + sandbox lifecycle. |
| **Seed agent** | Entry in `crates/server/src/seed.rs` with capabilities wired. |
| **Threat model** | Section in `specs/threat-model.md` covering integration-specific threats. |

New integrations should check off every row before merging. Existing integrations that are missing items should be brought up to parity incrementally.

## Integration Crates (`integrations/`)

Auto-registered via `inventory` plugin system. Each crate has a `SPEC.md`.

| Integration | Spec | Summary |
|---|---|---|
| Brave Search | [`integrations/brave-search/SPEC.md`](../integrations/brave-search/SPEC.md) | Web search via Brave Search API. Experimental (Dev only). |
| DuckDuckGo | [`integrations/duckduckgo/SPEC.md`](../integrations/duckduckgo/SPEC.md) | Instant answers via DuckDuckGo API. Experimental (Dev only). |
| Browserless | [`integrations/browserless/SPEC.md`](../integrations/browserless/SPEC.md) | Cloud browser automation — screenshots, DOM, scraping, multi-step interactions. REST and CDP modes. |
| E2B | [`integrations/e2b/SPEC.md`](../integrations/e2b/SPEC.md) | Cloud sandbox environments via E2B management API + envd runtime endpoints. Platform-owned token, multiple sandboxes per session. |
| Daytona | [`integrations/daytona/SPEC.md`](../integrations/daytona/SPEC.md) | Cloud sandbox environments via Daytona REST API. Multiple sandboxes per session. |
| Deno | [`integrations/deno/SPEC.md`](../integrations/deno/SPEC.md) | Cloud sandbox environments via Deno websocket sandbox API. Multiple sandboxes per session. |
| Sprites | [`integrations/sprites/SPEC.md`](../integrations/sprites/SPEC.md) | Persistent Firecracker microVMs via Sprites (Fly.io). Persistent filesystem, checkpoints, HTTP services. |
| Container Sandbox | [`specs/container-sandbox.md`](container-sandbox.md) | Self-hosted container sandbox via Docker Engine REST API. Configurable runtime (runc, sysbox, kata, gvisor). |
| Docker | `integrations/docker/` | Container-based agent execution. Experimental (Dev only). No spec yet. Superseded by Container Sandbox. |

## Messaging Integrations (`crates/server/`)

Platform adapters connecting agents to messaging channels. Uses the channel abstraction layer defined in [`specs/messaging-integrations.md`](messaging-integrations.md) (`InboundChannelEvent`, `ChannelDeliveryAdapter`, `SessionRoutingStrategy`, `ThreadContext`).

| Integration | Spec | Summary |
|---|---|---|
| Slack Bot | [`crates/server/specs/slack-integration.md`](../crates/server/specs/slack-integration.md) | Deploy agents as Slack bots. Uses `InboundChannelEvent` for parsing, `build_session_routing_tag()` for routing, `SlackDeliveryAdapter` implementing `ChannelDeliveryAdapter`. |

## Server Integrations (`crates/server/specs/`)

Embedded in the server crate.

| Integration | Spec | Summary |
|---|---|---|
| User Connections | [`crates/server/specs/user-connections.md`](../crates/server/specs/user-connections.md) | OAuth/API-key connections to GitHub, GitLab, Bitbucket, Daytona for repo and sandbox access. |
| Valkey Cache | [`crates/server/specs/cache.md`](../crates/server/specs/cache.md) | Distributed rate limiting via Valkey; in-process caching via `moka`. |

## Observability (`specs/`)

| Integration | Spec | Summary |
|---|---|---|
| Braintrust | [`specs/braintrust-integration.md`](braintrust-integration.md) | LLM observability — sends agentic loop events to Braintrust project logs. |
| OpenTelemetry | [`specs/otel-observability.md`](otel-observability.md) | Gen-AI semantic convention tracing for full agentic execution lifecycle. |

