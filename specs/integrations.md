# Integrations

Index of external service integrations. Each spec lives alongside the code that implements it.

## Integration Crates (`integrations/`)

Auto-registered via `inventory` plugin system. Each crate has a `SPEC.md`.

| Integration | Spec | Summary |
|---|---|---|
| Brave Search | [`integrations/brave-search/SPEC.md`](../integrations/brave-search/SPEC.md) | Web search via Brave Search API. Experimental (Dev only). |
| DuckDuckGo | [`integrations/duckduckgo/SPEC.md`](../integrations/duckduckgo/SPEC.md) | Instant answers via DuckDuckGo API. Experimental (Dev only). |
| Browserless | [`integrations/browserless/SPEC.md`](../integrations/browserless/SPEC.md) | Cloud browser automation — screenshots, DOM, scraping, multi-step interactions. REST and CDP modes. |
| Daytona | [`integrations/daytona/SPEC.md`](../integrations/daytona/SPEC.md) | Cloud sandbox environments via Daytona REST API. Multiple sandboxes per session. |
| Docker | `integrations/docker/` | Container-based agent execution. Experimental (Dev only). No spec yet. |

## Server Integrations (`crates/server/specs/`)

Embedded in the server crate.

| Integration | Spec | Summary |
|---|---|---|
| Slack Bot | [`crates/server/specs/slack-integration.md`](../crates/server/specs/slack-integration.md) | Deploy agents as Slack bots with per-app manifests and signing secret verification. |
| User Connections | [`crates/server/specs/user-connections.md`](../crates/server/specs/user-connections.md) | OAuth/API-key connections to GitHub, GitLab, Bitbucket, Daytona for repo and sandbox access. |
| Valkey Cache | [`crates/server/specs/cache.md`](../crates/server/specs/cache.md) | Distributed rate limiting via Valkey; in-process caching via `moka`. |

## Observability (`specs/`)

| Integration | Spec | Summary |
|---|---|---|
| Braintrust | [`specs/braintrust-integration.md`](braintrust-integration.md) | LLM observability — sends agentic loop events to Braintrust project logs. |
| OpenTelemetry | [`specs/otel-observability.md`](otel-observability.md) | Gen-AI semantic convention tracing for full agentic execution lifecycle. |

## Build Infrastructure

| Tool | Spec | Summary |
|---|---|---|
| sccache | [`specs/sccache.md`](sccache.md) | Shared S3 compile cache for Rust builds. |
