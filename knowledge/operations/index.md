# Infrastructure and operations

* [Production Deployment Specification](production-deployment.md) - Production deployment aggregation and reverse proxy contract.
* [Migrations Specification](migrations.md) - Database migration naming, squashing, ordering, conflict resolution.
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
* [Email Sending](email.md) - Internal email delivery abstraction.
* [Egress Service](egress.md) - Host-owned outbound network boundary and future gateway.
* [Utility LLM Service](utility-llm.md) - Internal utility LLM service for capability internals.
* [Voice Sessions](voice.md) - Voice Sessions.
* [Session Counts](session-counts.md) - Denormalized session counters and the reads they exist to keep cheap.
