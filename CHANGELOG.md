# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **⚠️ Important:** There is no automatic migration between versions. Each major/minor release requires a fresh database. Back up any data you need before upgrading.

## [Unreleased]

<!-- New changes go here. Use `/prepare-release X.Y.Z` to generate draft from commits. -->

## [0.4.0] - 2025-01-17

### Highlights

- **Organization-scoped Multitenancy** - Full tenant isolation with organization-based resource scoping
- **MCP Support** - Model Context Protocol integration (without Auth)
- **Push-based Work Scheduling** - Real-time task distribution replacing polling for durable execution
- **DEV_MODE** - Run without PostgreSQL using in-memory storage for quick development
- **Multimodality support for images** - Attach and process multiple images in messages

### What's Changed

- fix(ci): update Docker tag strategy for stable latest ([#287](https://github.com/everruns/everruns/pull/287)) by [@chaliy](https://github.com/chaliy)
- refactor: remove outdated decision comments from Cargo.toml ([#285](https://github.com/everruns/everruns/pull/285)) by [@chaliy](https://github.com/chaliy)
- fix(deps): address security vulnerabilities ([#284](https://github.com/everruns/everruns/pull/284)) by [@chaliy](https://github.com/chaliy)
- refactor(db): squash migrations into two files ([#283](https://github.com/everruns/everruns/pull/283)) by [@chaliy](https://github.com/chaliy)
- feat(release): add automated release workflow with CHANGELOG.md as source of truth ([#282](https://github.com/everruns/everruns/pull/282)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add capability mounting to session filesystem ([#281](https://github.com/everruns/everruns/pull/281)) by [@chaliy](https://github.com/chaliy)
- feat(tests): add agent integration tests for tool calls across providers ([#280](https://github.com/everruns/everruns/pull/280)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add agent preview mode to show final agent shape ([#276](https://github.com/everruns/everruns/pull/276)) by [@chaliy](https://github.com/chaliy)
- docs: refactor README for users, move dev setup to CONTRIBUTING ([#278](https://github.com/everruns/everruns/pull/278)) by [@chaliy](https://github.com/chaliy)
- feat: implement organization-scoped multitenancy ([#277](https://github.com/everruns/everruns/pull/277)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): add MCP agent support with virtual capabilities ([#275](https://github.com/everruns/everruns/pull/275)) by [@chaliy](https://github.com/chaliy)
- feat(images): add multi-image attachment support for messages ([#274](https://github.com/everruns/everruns/pull/274)) by [@chaliy](https://github.com/chaliy)
- refactor: remove database trigger, implement usage tracking in Rust ([#273](https://github.com/everruns/everruns/pull/273)) by [@chaliy](https://github.com/chaliy)
- fix: remove outdated Temporal reference from dev.sh ([#272](https://github.com/everruns/everruns/pull/272)) by [@chaliy](https://github.com/chaliy)
- feat(worker): implement push-based task notifications ([#270](https://github.com/everruns/everruns/pull/270)) by [@chaliy](https://github.com/chaliy)
- feat(durable): add PostgreSQL-backed load test benchmarks ([#268](https://github.com/everruns/everruns/pull/268)) by [@chaliy](https://github.com/chaliy)
- fix(worker): reduce poll interval from 1s to 100ms ([#262](https://github.com/everruns/everruns/pull/262)) by [@chaliy](https://github.com/chaliy)
- feat(models): add favorite LLM models support ([#265](https://github.com/everruns/everruns/pull/265)) by [@chaliy](https://github.com/chaliy)
- feat(durable): integrate circuit breaker for LLM provider protection ([#263](https://github.com/everruns/everruns/pull/263)) by [@chaliy](https://github.com/chaliy)
- feat(ui): update session status and usage via SSE in real-time ([#258](https://github.com/everruns/everruns/pull/258)) by [@chaliy](https://github.com/chaliy)
- feat(dev): add llmsim provider support and seed data ([#261](https://github.com/everruns/everruns/pull/261)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add per-agent capability configuration ([#260](https://github.com/everruns/everruns/pull/260)) by [@chaliy](https://github.com/chaliy)
- fix(dev): fix dev mode LLM errors and improve DX ([#259](https://github.com/everruns/everruns/pull/259)) by [@chaliy](https://github.com/chaliy)
- fix(ui): remove max-width constraint from agent edit page ([#256](https://github.com/everruns/everruns/pull/256)) by [@chaliy](https://github.com/chaliy)
- fix(ui): maintain input focus after sending chat message ([#257](https://github.com/everruns/everruns/pull/257)) by [@chaliy](https://github.com/chaliy)
- refactor(ui): replace durable polling with SSE streaming ([#254](https://github.com/everruns/everruns/pull/254)) by [@chaliy](https://github.com/chaliy)
- feat: add LLM token usage tracking and visualization ([#250](https://github.com/everruns/everruns/pull/250)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add SessionCard component with status display and info button ([#247](https://github.com/everruns/everruns/pull/247)) by [@chaliy](https://github.com/chaliy)
- feat(mcp): add MCP server registration and management ([#246](https://github.com/everruns/everruns/pull/246)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add message info icon with metadata tooltip ([#243](https://github.com/everruns/everruns/pull/243)) by [@chaliy](https://github.com/chaliy)
- feat(api,ui): add pagination to sessions API ([#244](https://github.com/everruns/everruns/pull/244)) by [@chaliy](https://github.com/chaliy)
- feat(ui): add centralized capability icons ([#237](https://github.com/everruns/everruns/pull/237)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.3.x → 0.4.0:** No automatic migration. This release includes schema changes for multitenancy and capabilities. Export agents via API, reset database, re-import.

## [0.3.0] - 2025-01-09

### Highlights

- **Durable Execution Engine** - Custom PostgreSQL-backed workflow engine replacing Temporal
- **CLI Tool** - Command-line interface for agent and session management
- **OpenTelemetry Integration** - Distributed tracing with gen-ai semantic conventions
- **SSE Events** - Real-time session status updates replacing polling

### What's Changed

- feat(durable): add custom durable execution engine (Phases 1-4) ([#154](https://github.com/everruns/everruns/pull/154)) by [@chaliy](https://github.com/chaliy)
- refactor(telemetry): implement event-listener-based OTel with gen-ai semantic conventions ([#161](https://github.com/everruns/everruns/pull/161)) by [@chaliy](https://github.com/chaliy)
- feat(cli): add everruns CLI for agent and session management ([#163](https://github.com/everruns/everruns/pull/163)) by [@chaliy](https://github.com/chaliy)
- feat(docs): add auto-generated API Reference from OpenAPI spec ([#164](https://github.com/everruns/everruns/pull/164)) by [@chaliy](https://github.com/chaliy)
- feat(capabilities): add fake demo tools and agents for warehouse, AWS, CRM, and financial operations ([#168](https://github.com/everruns/everruns/pull/168)) by [@chaliy](https://github.com/chaliy)
- feat(api): add agent import/export endpoints ([#172](https://github.com/everruns/everruns/pull/172)) by [@chaliy](https://github.com/chaliy)
- feat(api): add input validation for agent create/update/import ([#173](https://github.com/everruns/everruns/pull/173)) by [@chaliy](https://github.com/chaliy)
- feat(dev): add seed agent markdown files and upload-agents command ([#176](https://github.com/everruns/everruns/pull/176)) by [@chaliy](https://github.com/chaliy)

### Migration Notes

**0.2.x → 0.3.0:** No automatic migration. Export agents via API, reset database, re-import.

## [0.2.0] - 2024-12

### Highlights

- **Temporal Integration** - Workflow orchestration via Temporal
- **PostgreSQL Storage** - Database layer with SQLx
- **Management UI** - Next.js dashboard for agent management

### What's Changed

- Initial implementation with Temporal-based workflow orchestration
- Complete rewrite from early POC architecture

### Migration Notes

**0.1.x → 0.2.0:** Complete rewrite. Manual migration required.

## [0.1.0] - 2024-11

### Highlights

- Initial proof-of-concept release
- Basic agent execution with simple message handling

---

## Versioning Policy

- **Major versions** (1.0, 2.0): Breaking API changes, architectural shifts
- **Minor versions** (0.3, 0.4): New features, schema changes requiring fresh DB
- **Patch versions** (0.3.1): Bug fixes, no schema changes
