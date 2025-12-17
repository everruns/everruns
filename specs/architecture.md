# Architecture Specification

## Abstract

Everruns is a durable AI agent execution platform built on Rust and Temporal. It provides APIs for managing agents, threads, and runs with streaming event output via the AG-UI protocol. The architecture prioritizes durability, observability, and developer experience.

## Requirements

### Core Architecture

1. **Monorepo Structure**: Single repository with Cargo workspace containing multiple crates
2. **Crate Separation**:
   - `everruns-api` - HTTP API server (axum), SSE streaming, health endpoints
   - `everruns-worker` - Temporal worker, workflows, activities, LLM providers
   - `everruns-agent-loop` - DB-agnostic agentic loop abstraction (traits, executor, step decomposition)
   - `everruns-contracts` - DTOs, AG-UI events, OpenAPI schemas
   - `everruns-storage` - PostgreSQL (sqlx), migrations, repositories
3. **Frontend**: Next.js application in `apps/ui/` for management and chat interfaces

### Data Layer

1. **Database**: PostgreSQL 18+ with native UUID v7 support
2. **UUID Strategy**: All IDs use UUID v7 (time-ordered, better indexing, naturally sortable)
3. **Migrations**: Managed via sqlx-cli in `crates/everruns-storage/migrations/`

### Execution Layer

1. **Runner Abstraction**: `AgentRunner` trait provides pluggable execution backends
2. **Execution Modes**:
   - **In-Process** (`AGENT_RUNNER_MODE=inprocess`): Workflows run as tokio tasks in API process. Good for development.
   - **Temporal** (`AGENT_RUNNER_MODE=temporal`): True Temporal workflow execution with separate worker process
3. **Workflow Isolation**: Temporal concepts (workflow IDs, task queues) never exposed in public API
4. **Event Streaming**: AG-UI protocol over SSE for real-time event delivery via database-backed events

See [specs/temporal-integration.md](temporal-integration.md) for detailed Temporal architecture.

### Agent Loop Abstraction (`everruns-agent-loop`)

The agentic loop is encapsulated in a DB-agnostic crate with pluggable backends:

1. **Trait-Based Design**:
   - `EventEmitter` - Emit events during loop execution
   - `MessageStore` - Load/store conversation messages
   - `LlmProvider` - Call LLM with streaming support
   - `ToolExecutor` - Execute tool calls

2. **Execution Modes**:
   - **In-Process**: `AgentLoop::run()` - Complete loop execution in single process
   - **Decomposed**: `AgentLoop::execute_step()` - Step-by-step execution for Temporal integration

3. **Step Abstraction** (`step.rs`):
   - `StepInput` / `StepOutput` - Input/output for each step
   - `LoopStep` - Records for each step (setup, llm_call, tool_execution, finalize)
   - Enables each LLM call and tool call to be a separate Temporal activity

4. **In-Memory Implementations** (for testing/examples):
   - `InMemoryMessageStore`, `InMemoryEventEmitter`
   - `MockLlmProvider`, `MockToolExecutor`
   - `InMemoryAgentLoopBuilder` for easy test setup

### API Design

1. **RESTful**: Standard REST conventions for CRUD operations
2. **Versioning**: API versioned under `/v1/` prefix
3. **Documentation**: OpenAPI 3.0 with Swagger UI at `/swagger-ui/`

### Infrastructure

1. **Local Development**: Docker Compose in `harness/` for Postgres, Temporal, Temporal UI
2. **CI/CD**: GitHub Actions for format, lint, test, smoke test, Docker build
3. **License Compliance**: cargo-deny for dependency license checking
