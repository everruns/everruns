# Architecture V2 Proposal

## Abstract

This document proposes a new architecture where workers communicate with a control plane via gRPC instead of direct database access. It also reorganizes the crate structure for clearer separation of concerns.

## Goals

1. **Security isolation**: Workers don't need database credentials or encryption keys
2. **Clear boundaries**: Control plane owns all state; workers are stateless executors
3. **Simpler crates**: Fewer, more focused crates with clear responsibilities
4. **Industry alignment**: Follow patterns from Temporal, Kubernetes, Ray

## Current vs Proposed Architecture

### Current Architecture (v1)

```
┌─────────────────────┐                    ┌─────────────────────┐
│     API Process     │                    │   Worker Process    │
│                     │                    │                     │
│  TemporalRunner     │    Temporal        │  TemporalWorker     │
│  (starts workflows) │─────► Server ◄─────│  (polls tasks)      │
└─────────────────────┘                    └─────────────────────┘
         │                                           │
         │              ┌───────────┐                │
         └──────────────│ PostgreSQL │───────────────┘
                        │  (shared)  │
                        └────────────┘
```

**Problems:**
- Workers need database credentials
- Workers need encryption keys for LLM API keys
- Tight coupling between workers and storage layer
- Hard to run workers in untrusted environments

### Proposed Architecture (v2)

```
┌─────────────────────────────────────────────────────────────────┐
│                     Control Plane                                │
│                                                                  │
│  ┌─────────────────────┐    ┌─────────────────────────────────┐ │
│  │   REST API (axum)   │    │    gRPC Service (tonic)         │ │
│  │   :9000/v1/*        │    │    :50051 WorkerService         │ │
│  │   Public clients    │    │    Internal workers only        │ │
│  └──────────┬──────────┘    └──────────────┬──────────────────┘ │
│             │                              │                     │
│             └──────────────┬───────────────┘                     │
│                            │                                     │
│                    ┌───────┴───────┐                             │
│                    │   Services    │                             │
│                    │   Storage     │                             │
│                    │   (sqlx)      │                             │
│                    └───────┬───────┘                             │
│                            │                                     │
│                    ┌───────┴───────┐                             │
│                    │  PostgreSQL   │                             │
│                    └───────────────┘                             │
└─────────────────────────────────────────────────────────────────┘
         ▲                                    ▲
         │ REST                               │ gRPC
         │                                    │
    ┌────┴────┐                      ┌────────┴────────┐
    │ Web UI  │                      │     Workers     │
    │ Clients │                      │  (no DB access) │
    └─────────┘                      └─────────────────┘
                                              │
                                              │ Temporal
                                              ▼
                                     ┌─────────────────┐
                                     │ Temporal Server │
                                     └─────────────────┘
```

## Crate Reorganization

### Current Crate Structure

```
crates/
├── everruns-api/        # HTTP API, services, routes
├── everruns-worker/     # Temporal worker, workflows, activities
├── everruns-core/       # Domain types, traits, atoms, tools, capabilities
├── everruns-storage/    # Database layer (sqlx)
├── everruns-openai/     # OpenAI LLM driver
└── everruns-anthropic/  # Anthropic LLM driver
```

**Problems:**
- `everruns-core` is too big (types + traits + atoms + tools + capabilities)
- `everruns-storage` is separate but tightly coupled to api
- Worker needs storage crate for Db* adapters
- No clear schema contract between components

### Proposed Crate Structure

```
crates/
├── schemas/             # everruns-schemas: Shared type contracts
├── runtime/             # everruns-runtime: Agent execution (atoms, drivers, tools)
├── internal-protocol/   # everruns-internal-protocol: gRPC proto definitions
├── control-plane/       # everruns-control-plane: API + services + storage
├── worker/              # everruns-worker: Temporal worker (thin, uses runtime)
├── openai/              # everruns-openai: OpenAI driver
└── anthropic/           # everruns-anthropic: Anthropic driver
```

### Crate Details

#### `schemas` (everruns-schemas)
Shared type contracts used across all components. No business logic.

```rust
// What it contains:
pub mod agent;       // Agent, RuntimeAgent
pub mod session;     // Session, SessionStatus
pub mod message;     // Message, MessageRole, ContentPart
pub mod events;      // Event, EventData, EventContext
pub mod tools;       // ToolCall, ToolResult, ToolDefinition
pub mod llm;         // LlmProviderType, ModelWithProvider
pub mod files;       // SessionFile, FileInfo, FileStat

// Dependencies: minimal (serde, uuid, chrono)
// Used by: runtime, control-plane, worker, internal-protocol
```

#### `runtime` (everruns-runtime)
Agent execution engine. Used by workers to run agents.

```rust
// What it contains:
pub mod atoms;           // InputAtom, ReasonAtom, ActAtom
pub mod capabilities;    // CapabilityRegistry, builtin capabilities
pub mod drivers;         // LlmDriver trait, DriverRegistry
pub mod tools;           // ToolExecutor, ToolRegistry
pub mod traits;          // Store traits (for gRPC implementations)

// Dependencies: schemas, openai, anthropic, tokio, async-trait
// Used by: worker
// Does NOT depend on: storage, database, control-plane
```

#### `internal-protocol` (everruns-internal-protocol)
gRPC service definitions for worker ↔ control-plane communication.

```rust
// What it contains:
// - Proto files in proto/
// - Generated Rust code via tonic-build
// - WorkerService client and server traits

// Dependencies: tonic, prost, schemas
// Used by: control-plane (server), worker (client)
```

#### `control-plane` (everruns-control-plane)
Central service with REST API, gRPC service, storage, and business logic.

```rust
// What it contains:
pub mod api;         // REST routes (axum)
pub mod grpc;        // gRPC WorkerService implementation
pub mod services;    // Business logic
pub mod storage;     // Database layer (merged from everruns-storage)
pub mod migrations;  // SQL migrations

// Dependencies: schemas, internal-protocol, axum, tonic, sqlx
// Binary: everruns-control-plane (single binary for API + gRPC)
```

#### `worker` (everruns-worker)
Thin Temporal worker that uses runtime for execution.

```rust
// What it contains:
pub mod workflow;    // TurnWorkflow state machine
pub mod activities;  // Activity implementations (call runtime atoms)
pub mod grpc_stores; // gRPC-backed trait implementations

// Dependencies: schemas, runtime, internal-protocol, temporal-sdk-core
// Binary: everruns-worker
// Does NOT depend on: storage, sqlx, database
```

### Dependency Graph

```
                    ┌──────────────┐
                    │   schemas    │
                    └──────┬───────┘
                           │
          ┌────────────────┼────────────────┐
          │                │                │
          ▼                ▼                ▼
    ┌──────────┐    ┌──────────────┐  ┌──────────────────┐
    │  openai  │    │  anthropic   │  │ internal-protocol│
    └────┬─────┘    └──────┬───────┘  └────────┬─────────┘
         │                 │                   │
         └────────┬────────┘                   │
                  │                            │
                  ▼                            │
            ┌──────────┐                       │
            │ runtime  │                       │
            └────┬─────┘                       │
                 │                             │
         ┌───────┴───────┐                     │
         │               │                     │
         ▼               ▼                     │
   ┌──────────┐   ┌──────────────┐             │
   │  worker  │   │control-plane │◄────────────┘
   └──────────┘   └──────────────┘
```

## gRPC WorkerService

### Service Definition

```protobuf
// proto/worker_service.proto
syntax = "proto3";
package everruns.worker.v1;

service WorkerService {
  // Agent/Session config (read-only)
  rpc GetAgent(GetAgentRequest) returns (GetAgentResponse);
  rpc GetSession(GetSessionRequest) returns (GetSessionResponse);

  // Messages (read/write)
  rpc AddMessage(AddMessageRequest) returns (AddMessageResponse);
  rpc GetMessage(GetMessageRequest) returns (GetMessageResponse);
  rpc ListMessages(ListMessagesRequest) returns (ListMessagesResponse);

  // Events (write with streaming option)
  rpc EmitEvent(EmitEventRequest) returns (EmitEventResponse);
  rpc EmitEventStream(stream EmitEventRequest) returns (stream EmitEventResponse);

  // LLM Providers (returns decrypted keys)
  rpc GetModelProvider(GetModelProviderRequest) returns (GetModelProviderResponse);
  rpc GetDefaultModel(GetDefaultModelRequest) returns (GetModelProviderResponse);

  // Session Files
  rpc ReadFile(ReadFileRequest) returns (ReadFileResponse);
  rpc WriteFile(WriteFileRequest) returns (WriteFileResponse);
  rpc DeleteFile(DeleteFileRequest) returns (DeleteFileResponse);
  rpc ListDirectory(ListDirectoryRequest) returns (ListDirectoryResponse);
  rpc StatFile(StatFileRequest) returns (StatFileResponse);
  rpc GrepFiles(GrepFilesRequest) returns (GrepFilesResponse);
  rpc CreateDirectory(CreateDirectoryRequest) returns (CreateDirectoryResponse);
}
```

### Worker gRPC Adapters

```rust
// crates/worker/src/grpc_stores.rs

/// gRPC-backed MessageStore for workers
pub struct GrpcMessageStore {
    client: WorkerServiceClient<Channel>,
}

#[async_trait]
impl MessageStore for GrpcMessageStore {
    async fn add(&self, session_id: Uuid, input: InputMessage) -> Result<Message> {
        let response = self.client.clone()
            .add_message(AddMessageRequest {
                session_id: Some(session_id.into()),
                input: Some(input.into()),
            })
            .await?;

        response.into_inner().message
            .ok_or_else(|| Error::NotFound)?
            .try_into()
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let response = self.client.clone()
            .list_messages(ListMessagesRequest {
                session_id: Some(session_id.into()),
                offset: 0,
                limit: 10000,
            })
            .await?;

        response.into_inner().messages
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }
}

// Similar implementations for:
// - GrpcAgentStore
// - GrpcSessionStore
// - GrpcEventEmitter
// - GrpcLlmProviderStore
// - GrpcSessionFileStore
```

## Industry Precedent

| System | Use Case | Communication Pattern |
|--------|----------|----------------------|
| **Temporal** | Workflow orchestration | Workers poll server via gRPC |
| **Kubernetes** | Container orchestration | kubelet ↔ API server via gRPC |
| **Ray** | Distributed computing | Worker ↔ head node via gRPC |
| **Envoy/Istio** | Service mesh | xDS protocol over gRPC |

**Why these systems chose gRPC:**
- 7-10x faster than REST for high-frequency operations
- Binary serialization (protobuf) reduces bandwidth
- HTTP/2 multiplexing reduces connection overhead
- Streaming enables efficient event delivery

**Relevance:** Workers already have gRPC dependencies via Temporal SDK (`tonic`, `prost`). Adding WorkerService has zero new dependency overhead.

## Implementation Plan

### Phase 1: Create schemas crate
**Goal:** Extract shared types into standalone crate

1. Create `crates/schemas/` with types from `everruns-core`
2. Move: `Agent`, `Session`, `Message`, `Event`, `ContentPart`, `ToolCall`, etc.
3. Update all crates to depend on `schemas`
4. Keep `everruns-core` temporarily as re-export layer for compatibility

**Deliverable:** All crates compile, tests pass, schemas crate exists

### Phase 2: Create runtime crate
**Goal:** Extract agent execution logic

1. Create `crates/runtime/` with atoms and capabilities from `everruns-core`
2. Move: `InputAtom`, `ReasonAtom`, `ActAtom`, capabilities, `ToolRegistry`
3. Move LLM driver registry integration
4. Define store traits in runtime (implementations in consumer crates)

**Deliverable:** Runtime crate with atoms, worker uses runtime for execution

### Phase 3: Create internal-protocol crate
**Goal:** Define gRPC contract

1. Create `crates/internal-protocol/` with proto files
2. Set up `tonic-build` in `build.rs`
3. Generate Rust types from proto definitions
4. Export client and server traits

**Deliverable:** Proto compiles, types available for both sides

### Phase 4: Add gRPC to control-plane
**Goal:** Control plane serves WorkerService

1. Rename `crates/everruns-api/` → `crates/control-plane/`
2. Merge `crates/everruns-storage/` into control-plane
3. Implement `WorkerService` gRPC server
4. Run REST (:9000) and gRPC (:50051) in same binary

**Deliverable:** Control plane serves both REST and gRPC

### Phase 5: Worker uses gRPC
**Goal:** Workers communicate via gRPC only

1. Implement `Grpc*Store` adapters in worker crate
2. Remove direct database dependencies from worker
3. Configure worker to connect to control-plane gRPC
4. Remove `everruns-storage` dependency from worker

**Deliverable:** Worker has no database access, uses gRPC for all operations

### Phase 6: Cleanup
**Goal:** Remove old code and crates

1. Remove `crates/everruns-core/` (replaced by schemas + runtime)
2. Remove `crates/everruns-storage/` (merged into control-plane)
3. Rename crate folders (remove `everruns-` prefix from folders)
4. Update documentation and CI

**Deliverable:** Clean crate structure matching proposal

## Migration Checklist

### Phase 1: schemas
- [ ] Create `crates/schemas/Cargo.toml`
- [ ] Move types: Agent, Session, Message, Event, ContentPart, ToolCall, ToolResult
- [ ] Move types: MessageRole, SessionStatus, LlmProviderType
- [ ] Move types: FileInfo, FileStat, SessionFile, GrepMatch
- [ ] Add serde derives with `#[serde(rename_all = "snake_case")]`
- [ ] Update dependent crates to use schemas
- [ ] Ensure all tests pass

### Phase 2: runtime
- [ ] Create `crates/runtime/Cargo.toml`
- [ ] Move atoms: InputAtom, ReasonAtom, ActAtom, AtomContext
- [ ] Move capabilities: CapabilityRegistry, builtin capabilities
- [ ] Move tool execution: ToolRegistry, ToolExecutor trait
- [ ] Move LLM driver integration
- [ ] Define trait bounds (MessageStore, EventEmitter, etc.)
- [ ] Worker depends on runtime
- [ ] Ensure all tests pass

### Phase 3: internal-protocol
- [ ] Create `crates/internal-protocol/Cargo.toml`
- [ ] Create `proto/worker_service.proto`
- [ ] Set up `build.rs` with tonic-build
- [ ] Verify proto compiles
- [ ] Export client/server types

### Phase 4: control-plane
- [ ] Rename `everruns-api` → `control-plane`
- [ ] Merge storage modules into control-plane
- [ ] Implement `WorkerService` trait
- [ ] Add gRPC server startup alongside REST
- [ ] Test gRPC endpoints with grpcurl
- [ ] Ensure REST API unchanged

### Phase 5: worker gRPC
- [ ] Implement GrpcMessageStore
- [ ] Implement GrpcAgentStore
- [ ] Implement GrpcSessionStore
- [ ] Implement GrpcEventEmitter
- [ ] Implement GrpcLlmProviderStore
- [ ] Implement GrpcSessionFileStore
- [ ] Remove sqlx dependency from worker
- [ ] Configure GRPC_ENDPOINT env var
- [ ] Run smoke tests with gRPC communication

### Phase 6: cleanup
- [ ] Remove everruns-core crate
- [ ] Remove everruns-storage crate
- [ ] Rename folder: `everruns-worker` → `worker`
- [ ] Rename folder: `everruns-openai` → `openai`
- [ ] Rename folder: `everruns-anthropic` → `anthropic`
- [ ] Update CLAUDE.md and documentation
- [ ] Update CI workflows
- [ ] Final smoke test

## Configuration Changes

### Control Plane

```bash
# Existing
DATABASE_URL=postgres://...
API_PORT=9000

# New
GRPC_PORT=50051
```

### Worker

```bash
# Remove
DATABASE_URL=postgres://...        # No longer needed
ENCRYPTION_KEY=...                 # No longer needed

# Add
CONTROL_PLANE_GRPC_URL=http://control-plane:50051
WORKER_AUTH_TOKEN=...              # For gRPC authentication
```

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| gRPC latency | Profile early; gRPC is ~50% faster than REST |
| Control plane becomes bottleneck | Horizontal scaling, connection pooling |
| Network failures | Retry with backoff, circuit breaker |
| Migration complexity | Phase-based approach, feature flags |
| Proto schema evolution | Use proto3 optional fields, never remove fields |

## Success Criteria

1. **Workers have no database access**: No sqlx/postgres dependencies
2. **Single control-plane binary**: Serves REST + gRPC
3. **Smoke tests pass**: Full workflow works over gRPC
4. **Latency acceptable**: < 5ms overhead per gRPC call
5. **Clean crate structure**: 5 crates with clear responsibilities
