# Architecture Specification

## Abstract

Everruns is a durable AI agent execution platform built on Rust and Temporal. It provides APIs for managing agents, threads, and runs with streaming event output via the AG-UI protocol. The architecture prioritizes durability, observability, and developer experience.

## Requirements

### Core Architecture

1. **Monorepo Structure**: Single repository with Cargo workspace containing multiple crates
2. **Crate Separation**:
   - `everruns-api` - HTTP API server (axum), SSE streaming, health endpoints
   - `everruns-worker` - Temporal worker, workflows, activities, LLM providers
   - `everruns-contracts` - DTOs, AG-UI events, OpenAPI schemas
   - `everruns-storage` - PostgreSQL (sqlx), migrations, repositories
3. **Frontend**: Next.js application in `apps/ui/` for management and chat interfaces

### Data Layer

1. **Database**: PostgreSQL 18+ with native UUID v7 support
2. **UUID Strategy**: All IDs use UUID v7 (time-ordered, better indexing, naturally sortable)
3. **Migrations**: Managed via sqlx-cli in `crates/everruns-storage/migrations/`

### Execution Layer

1. **Temporal**: Used as internal implementation detail for durable workflow execution
2. **Workflow Isolation**: Temporal concepts (workflow IDs, task queues) never exposed in public API
3. **Event Streaming**: AG-UI protocol over SSE for real-time event delivery

### API Design

1. **RESTful**: Standard REST conventions for CRUD operations
2. **Versioning**: API versioned under `/v1/` prefix
3. **Documentation**: OpenAPI 3.0 with Swagger UI at `/swagger-ui/`

### Infrastructure

1. **Local Development**: Docker Compose in `harness/` for Postgres, Temporal, Temporal UI
2. **CI/CD**: GitHub Actions for format, lint, test, smoke test, Docker build
3. **License Compliance**: cargo-deny for dependency license checking
