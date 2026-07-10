# Specs index

`specs/` contains durable design intent, constraints, and feature contracts. Use this map to find the relevant spec before changing behavior. Integration specs live near their crates; see `specs/integrations.md`.

Specs capture the "why" and "what", not exhaustive source detail. Link to code for full fields, enum variants, exact API shapes, or SQL DDL instead of duplicating them.

## Core

- `specs/concepts.md` - Core entities, relationships, and concept diagram
- `specs/architecture.md` - System architecture, crate structure, infrastructure
- `specs/code-organization.md` - Developer conventions: formatting, testing, error handling, UI patterns
- `specs/models.md` - Data models (Agent, Session, Message, etc.)
- `specs/id-schema.md` - Standardized prefixed ID format
- `specs/domains.md` - Domain modules: Command trait, feature-oriented structure, MCP catalog generation
- `specs/runtime.md` - Public in-process runtime contract for embedded execution
- `specs/embedding.md` - Embedding contract and `PlatformDefinition`
- `specs/providers.md` - Providers domain model: drivers, services, providers, models, model profiles
- `specs/llm-drivers.md` - LLM driver trait, provider implementations
- `specs/cli.md` - CLI specification

## APIs and execution

- `specs/apis.md` - HTTP API endpoints, error handling
- `specs/api-conventions.md` - Cross-cutting HTTP API conventions
- `specs/api-streaming.md` - SSE streaming conventions for API endpoints
- `specs/api-examples.md` - Per-operation request/response examples on `#[utoipa::path]` handlers
- `specs/api-llm-extensions.md` - LLM-specific OpenAPI extensions (`x-llm-*`)
- `specs/api-sdk-extensions.md` - SDK model semantics in OpenAPI (`x-sdk-*`)
- `specs/public-endpoints.md` - Public endpoints, error sanitization contract, stable public code set
- `specs/error-disclosure.md` - Semantic driver error kinds and session error-disclosure modes
- `specs/events.md` - Event types, SSE streaming, contract and compatibility guarantees
- `specs/execution-phases.md` - Execution phases (Commentary/FinalAnswer) for multi-step tool flows
- `specs/tool-execution.md` - Tool types and execution flow
- `specs/tool-narration.md` - Backend-authored, argument-aware narration for common tool families
- `specs/tool-output-distillation.md` - Content-aware distillation of large non-exec tool results at capture time
- `specs/capabilities.md` - Agent capabilities system
- `specs/guardrails.md` - Guardrails (capability-based output/tool-call checks)
- `specs/background-execution.md` - `background_execution` capability and cross-cutting / auto-activation contract
- `specs/client-side-tools.md` - Client-side tools for API/SDK consumers
- `specs/tool-search.md` - OpenAI tool_search deferred tool loading capability
- `specs/fetchkit.md` - fetchkit library powering the `web_fetch` capability
- `specs/toolkit-library-contract.md` - Convention for external toolkit libraries
- `specs/bashkit-requirements.md` - Bash sandbox capabilities and requirements
- `specs/lua-execution.md` - Experimental Lua execution capability (sandboxed VFS scripting; aims to supersede bashkit_shell)

## Agents, sessions, and runtime resources

- `specs/agent-instructions.md` - AGENTS.md support (dynamic project instructions)
- `specs/agent-identities.md` - Agent identities (virtual principals for unattended execution)
- `specs/agent-blueprints.md` - Pre-built agent definitions
- `specs/agent-versions.md` - Immutable Agent configuration snapshots
- `specs/agent-handoff.md` - Agent handoff behavior
- `specs/user-hooks.md` - User-authored lifecycle hooks for agent execution
- `specs/agent-reliability-tests.md` - Agent execution reliability tests
- `specs/subagents.md` - Subagent orchestration
- `specs/session-tasks.md` - Session task registry for background work
- `specs/session-resources.md` - Session resource registry
- `specs/leased-resources.md` - Generic lease primitive
- `specs/session-sandbox.md` - Managed session-owned sandbox capability and lifecycle
- `specs/workspace.md` - Session workspace (file surface + tables)
- `specs/file-store.md` - Pluggable `SessionFileStore` backends
- `specs/object-storage.md` - Optional S3-compatible blob backend for file/image content
- `specs/session-sqldb.md` - Session-scoped SQL databases
- `specs/session-export.md` - Session export to JSONL
- `specs/forking-sessions.md` - Fork a session into an independent copy (history + workspace + state)
- `specs/knowledge-bases.md` - Curated organization knowledge
- `specs/okf-adoption.md` - Open Knowledge Format (OKF) import/export adoption for Knowledge Bases
- `specs/knowledge-indexes.md` - Source-backed, embedded, citable knowledge indexes
- `specs/memory.md` - Org-scoped named Memories (mountable into Workspaces)
- `specs/infinity-context.md` - Unlimited conversation length via context management
- `specs/compaction.md` - Context compaction capability
- `specs/client-hints.md` - Generic client hints mechanism

## UI and generative UI

- `specs/markdown-messages.md` - Chat message markdown rendering with llm-ui
- `specs/openui.md` - OpenUI generative-UI capability
- `specs/a2ui.md` - A2UI generative-UI capability
- `specs/mcp-cards.md` - MCP Apps entity cards and sandboxed HTML resources
- `specs/brand.md` - Brand identity, colors, typography
- `specs/diagrams.md` - Diagram specification
- `specs/documentation.md` - Documentation site
- `specs/signup-experience-redesign-brief.md` - Design brief for on-brand sign-up / onboarding screens

## MCP, integrations, and apps

- `specs/mcp.md` - MCP server endpoint, OAuth 2.1 authentication, protocol, security
- `specs/mcp-servers.md` - MCP client remote server registration, CRUD API, tool naming, execution
- `specs/runtime-mcp.md` - MCP client in the in-process runtime: shared `everruns-mcp` crate, transport abstraction (HTTP + optional stdio), pluggable auth
- `specs/integrations.md` - Integration specs index
- `specs/apps.md` - Apps system
- `specs/public-chat.md` - Public Chat (hosted, isolated chat app) — product spec/proposal
- `specs/app-invocation-channels.md` - App schedule/webhook invocation channels
- `specs/app-endpoint-auth.md` - Shared inbound auth framework for App-published endpoints
- `specs/app-api-keys.md` - App-scoped, execution-only API keys over the native session API
- `specs/a2a-channel.md` - A2A inbound channel
- `specs/a2a-capability.md` - A2A outbound delegation capability
- `specs/fcp-channel.md` - FCP inbound channel
- `specs/messaging-integrations.md` - Messaging integrations
- `specs/everruns-dev-plugin.md` - Everruns(Dev) plugin sync contract
- `specs/plugins.md` - Plugin host: marketplaces and cross-host plugin packages installed as capabilities
- `specs/model-router.md` - Model Routers

## Infrastructure and operations

- `specs/production-deployment.md` - Production deployment aggregation and reverse proxy contract
- `specs/migrations.md` - Database migration naming, squashing, ordering, conflict resolution
- `specs/durable-execution-engine.md` - PostgreSQL-backed durable workflow engine
- `specs/scheduled-tasks.md` - Cron-based scheduled tasks
- `specs/prometheus-metrics.md` - Prometheus `/metrics` endpoint and scrape configuration
- `specs/observability.md` - Observability providers
- `specs/correlation-ids.md` - Correlation IDs
- `specs/load-testing.md` - End-to-end load testing framework
- `specs/network-access.md` - Network access allowlist/blocklist
- `specs/system-allowlist.md` - System-wide outbound allowlist ("green list")
- `specs/localization.md` - Locale/timezone resolution and backend localization rules
- `specs/notifications.md` - Generic user notifications
- `specs/email.md` - Internal email delivery abstraction
- `specs/egress.md` - Host-owned outbound network boundary and future gateway
- `specs/utility-llm.md` - Internal utility LLM service for capability internals
- `specs/voice.md` - Voice Sessions

## Security, auth, and governance

- `specs/authentication.md` - Authentication modes and OAuth
- `specs/encryption.md` - Envelope encryption for sensitive data
- `specs/audit-logging.md` - Audit logging
- `specs/threat-model.md` - Security threat model
- `specs/security-testing.md` - Security testing process (threat-model tests, fail-rs, DeepSec, supply chain)
- `specs/multitenancy.md` - Organization-based multitenancy
- `specs/permissions.md` - Fine-grained permissions model
- `specs/feature-flags.md` - Feature flags system
- `specs/budgeting.md` - Extensible budgeting system
- `specs/usage-tracking.md` - LLM token usage tracking
- `specs/machine-payments.md` - Capability-side payments to external paid services

## Evaluation, testing, and reporting

- `specs/test-cases.md` - Manual test case format
- `specs/evals.md` - User-facing behavioral evals
- `specs/agent-checks.md` - Advisory agent config checks (lint, LLM analysis, health checks)
- `specs/online-evals.md` - Online evals (Observers) over production sessions (proposed)
- `specs/swe-bench-lite.md` - SWE-bench Lite evaluation harness
- `specs/reporting.md` - Async reporting
- `specs/dataset-export.md` - Reward-labeled trajectory dataset export from eval runs
- `specs/atif-adoption.md` - ATIF (Agent Trajectory Interchange Format) export/import adoption
- `specs/reporting-backends.md` - Phase 3 reference evaluation
- `specs/fail-rs-testing.md` - Failure injection testing with fail-rs

## Project workflow

- `specs/shipping.md` - Goal-oriented shipping and merge-readiness guidance
- `specs/maintenance.md` - Goal-oriented maintenance and release-readiness guidance
- `specs/release-process.md` - Release workflow with CHANGELOG.md
- `specs/issue-tracking.md` - Issue tracking (Linear, OSS project)
- `specs/skills-registry.md` - Agent Skills registry
- `specs/commands.md` - Slash commands system
- `specs/xml-prompt-formatting.md` - XML tags for system prompt structure
- `specs/dismissed-options.md` - Technical options considered but dismissed

## Harnesses and sandboxes

- `specs/harness-types.md` - Built-in harness types
- `specs/coding-session-sandbox-harness.md` - Built-in coding harness using managed session sandbox
- `specs/coding-daytona-harness.md` - Built-in coding harness backed by Daytona cloud sandboxes
