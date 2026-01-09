# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **⚠️ Important:** There is no automatic migration between versions. Each major/minor release requires a fresh database. Back up any data you need before upgrading.

## [0.3.0] - 2025-01-09

### Added

- **Durable Execution Engine** - Custom workflow engine replacing Temporal dependency
  - Event-sourced workflow instances with replay capability
  - Distributed task queue optimized for 1000+ concurrent workers
  - Dead letter queue for failed tasks
  - Worker registry with backpressure signaling
  - Circuit breaker state for external service protection
- **CLI Tool** (`everruns`) - Command-line interface for agent and session management
- **Agent Import/Export** - YAML-based agent definitions with import/export API endpoints
- **SSE Events** - Real-time session status updates via Server-Sent Events (replaces polling)
- **OpenTelemetry Integration** - Distributed tracing with Jaeger and gen-ai semantic conventions
- **Management UI** - Improved session interface with URL-based routing and SSE support
- **API Reference Docs** - Auto-generated from OpenAPI spec via Starlight
- **LlmSim Driver** - Testing driver for integration tests without real LLM calls
- **Demo Capabilities** - Fake tools for warehouse, AWS, CRM, and financial operations

### Changed

- **Architecture** - Central service layer with gRPC-only worker communication
- **Event Protocol** - Standardized typed EventData with turn-based workflow
- **Session Status** - New lifecycle: `started` → `active` → `idle` (replaces pending/running/completed/failed)
- **Database Schema** - Squashed to two migrations (base + durable)

### Technical

- Migrated telemetry to event-listener-based OpenTelemetry
- Unified core DTOs and improved type safety
- Added input validation for agent create/update operations
- Moved LLM drivers to provider-specific crates with DriverRegistry pattern

## [0.2.0] - 2024-12

### Added

- **Temporal Integration** - Workflow orchestration via Temporal
- **Basic Agent Loop** - Core agentic execution with tool calling
- **PostgreSQL Storage** - Database layer with SQLx
- **HTTP API** - RESTful endpoints for agents, sessions, messages
- **Management UI** - Next.js dashboard for agent management

### Changed

- Complete rewrite from early POC architecture

## [0.1.0] - 2024-11

### Added

- Initial proof-of-concept
- Basic agent execution
- Simple message handling

---

## Versioning Policy

- **Major versions** (1.0, 2.0): Breaking API changes, architectural shifts
- **Minor versions** (0.3, 0.4): New features, schema changes requiring fresh DB
- **Patch versions** (0.3.1): Bug fixes, no schema changes

## Migration Notes

**0.2.x → 0.3.0:** No automatic migration. Export agents via API, reset database, re-import.

**0.1.x → 0.2.0:** Complete rewrite. Manual migration required.
