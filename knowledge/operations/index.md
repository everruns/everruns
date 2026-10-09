# Infrastructure and operations

* [Production Deployment Specification](production-deployment.md) - Production deployment aggregation and reverse proxy contract.
* [Migrations Specification](migrations.md) - Database migration naming, squashing, ordering, conflict resolution.
* [Caching and Distributed Rate Limiting](cache.md) - Valkey rate limiting and in-process moka caching.
* [Durable Execution Engine Specification](durable-execution-engine.md) - PostgreSQL-backed durable workflow engine.
* [Scheduled Tasks Specification](scheduled-tasks.md) - Cron-based scheduled tasks.
* [Prometheus Metrics Endpoint](prometheus-metrics.md) - Prometheus `/metrics` endpoint and scrape configuration.
* [Observability Providers](observability.md) - Observability providers.
* [Correlation IDs](correlation-ids.md) - Correlation IDs.
* [Load Testing Specification](load-testing.md) - End-to-end load testing framework.
* [Network Access List](network-access.md) - Network access allowlist/blocklist.
* [System-wide Outbound Allowlist](system-allowlist.md) - System-wide outbound allowlist ("green list").
* [Localization And Timezone Resolution](localization.md) - Locale/timezone resolution and backend localization rules.
* [Notifications](notifications.md) - Generic user notifications.
* [Actionable Health Issues](health-issues.md) - Persistent operational issues and verified recovery through notifications.
* [Email Sending](email.md) - Internal email delivery abstraction.
* [Egress Service](egress.md) - Host-owned outbound network boundary and future gateway.
* [Utility LLM Service](utility-llm.md) - Internal utility LLM service for capability internals.
* [Decisions Service](decisions-service.md) - Provider-bound decisions and separate utility authority.
* [OpenRouter Decisions Proposal](openrouter-decisions-proposal.md) - Provider service and catalog design.
* [Voice Channels on the Platform Server](voice.md) - How the server runs voice calls: voice channel type, call routes, delegated voice loop, leases and events.
* [Session Counts](session-counts.md) - Denormalized session counters and the reads they exist to keep cheap.
