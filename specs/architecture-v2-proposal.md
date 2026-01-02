# Architecture V2 Proposal

## Abstract

This document proposes a new architecture where workers communicate with a control plane via gRPC instead of direct database access. It also reorganizes the crate structure for clearer separation of concerns.

## Goals

1. **Deployment simplification**: Workers don't need database credentials or encryption keys
2. **Clear boundaries**: Control plane owns all state; workers are stateless executors
3. **Simpler crates**: Fewer, more focused crates with clear responsibilities
4. **Future investment**: Foundation for scaling, multi-tenancy, and security enhancements

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Worker ↔ Control Plane protocol | gRPC | Industry standard, already in stack via Temporal |
| LLM calls | Workers call directly | Simpler, trust workers for now |
| Tool execution | Workers execute locally | Direct, can split later if needed |
| Temporal access | Direct from workers | Acceptable, not sensitive |
| Schema source of truth | Rust types | Full ergonomics, proto mirrors |
| OpenAPI support | utoipa in schemas (feature flag) | Reuse types in REST API |
| Migration strategy | Big bang | No backward compatibility needed |

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
- `everruns-core` is a monolith (types + atoms + tools + capabilities)

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
                                              │ Temporal (direct)
                                              ▼
                                     ┌─────────────────┐
                                     │ Temporal Server │
                                     └─────────────────┘
```

## Crate Reorganization

### Current → Proposed

```
CURRENT                              PROPOSED
crates/                              crates/
├── everruns-api/        ──┐         ├── schemas/             # everruns-schemas
├── everruns-storage/    ──┴──────►  ├── runtime/             # everruns-runtime
├── everruns-core/       ──┬──────►  ├── internal-protocol/   # everruns-internal-protocol
│                          │         ├── control-plane/       # everruns-control-plane
├── everruns-worker/     ──┴──────►  ├── worker/              # everruns-worker
├── everruns-openai/     ─────────►  ├── openai/              # everruns-openai
└── everruns-anthropic/  ─────────►  └── anthropic/           # everruns-anthropic
```

**Naming convention:**
- Folder: `schemas/`, `runtime/`, etc. (no prefix)
- Package name: `everruns-schemas`, `everruns-runtime`, etc.
- Binary name: `everruns-control-plane`, `everruns-worker`

### Crate Details

#### `schemas` (everruns-schemas)

Shared type contracts. Source of truth for all data structures.

```rust
// What it contains:
pub mod agent;       // Agent, RuntimeAgent
pub mod session;     // Session, SessionStatus
pub mod message;     // Message, MessageRole, ContentPart
pub mod events;      // Event, EventData, EventContext
pub mod tools;       // ToolCall, ToolResult, ToolDefinition
pub mod llm;         // LlmProviderType, ModelWithProvider
pub mod files;       // SessionFile, FileInfo, FileStat

// Features:
// - "openapi" enables utoipa::ToSchema derives

// Dependencies: serde, uuid, chrono, utoipa (optional)
// Used by: ALL other crates
```

#### `runtime` (everruns-runtime)

Agent execution engine with atoms, capabilities, and tools.

```rust
// What it contains:
pub mod atoms;           // InputAtom, ReasonAtom, ActAtom, AtomContext
pub mod capabilities;    // CapabilityRegistry + all builtin capabilities
│   ├── file_system.rs
│   ├── web_fetch.rs
│   ├── current_time.rs
│   └── ...
pub mod drivers;         // LlmDriver trait, DriverRegistry
pub mod tools;           // ToolExecutor trait, ToolRegistry
pub mod traits;          // MessageStore, EventEmitter, AgentStore, etc.

// Dependencies: schemas, openai, anthropic, tokio, async-trait
// Used by: worker
// Does NOT depend on: control-plane, sqlx, database
```

#### `internal-protocol` (everruns-internal-protocol)

gRPC contract between workers and control plane. Proto mirrors Rust schemas.

```rust
// What it contains:
// - proto/worker_service.proto
// - Generated Rust code via tonic-build
// - Conversion traits: Proto ↔ Rust schemas

// Dependencies: tonic, prost, schemas
// Used by: control-plane (server), worker (client)
```

#### `control-plane` (everruns-control-plane)

Central service: REST API + gRPC service + storage.

```rust
// What it contains:
pub mod api;         // REST routes (axum), OpenAPI
pub mod grpc;        // gRPC WorkerService implementation
pub mod services;    // Business logic
pub mod storage;     // Database layer (sqlx) - merged from everruns-storage
│   ├── repositories.rs
│   ├── models.rs
│   └── migrations/

// Dependencies: schemas, internal-protocol, runtime (for types), axum, tonic, sqlx
// Binary: everruns-control-plane
```

#### `worker` (everruns-worker)

Thin Temporal worker. Executes atoms via runtime, talks to control-plane via gRPC.

```rust
// What it contains:
pub mod workflow;      // TurnWorkflow state machine
pub mod activities;    // Activity implementations
pub mod grpc_client;   // gRPC client wrapper
pub mod stores;        // GrpcMessageStore, GrpcAgentStore, etc.

// Dependencies: schemas, runtime, internal-protocol, temporal-sdk-core
// Binary: everruns-worker
// Does NOT have: sqlx, database access
```

#### `openai` / `anthropic` (everruns-openai, everruns-anthropic)

LLM provider implementations. Unchanged, stay separate.

```rust
// Dependencies: schemas (for types), reqwest, tokio
// Used by: runtime
```

### Dependency Graph

```
                         ┌──────────────┐
                         │   schemas    │
                         └──────┬───────┘
                                │
         ┌──────────────────────┼──────────────────────┐
         │                      │                      │
         ▼                      ▼                      ▼
   ┌──────────┐          ┌──────────────┐    ┌──────────────────┐
   │  openai  │          │  anthropic   │    │ internal-protocol│
   └────┬─────┘          └──────┬───────┘    └────────┬─────────┘
        │                       │                     │
        └───────────┬───────────┘                     │
                    │                                 │
                    ▼                                 │
              ┌──────────┐                            │
              │ runtime  │                            │
              └────┬─────┘                            │
                   │                                  │
           ┌───────┴───────┐                          │
           │               │                          │
           ▼               ▼                          │
     ┌──────────┐   ┌──────────────┐                  │
     │  worker  │   │control-plane │◄─────────────────┘
     └────┬─────┘   └──────────────┘
          │
          │ gRPC
          └──────────────────────────────────────────►
```

## gRPC WorkerService

### Batched RPCs for Efficiency

Instead of multiple calls per turn, use batched endpoints:

```protobuf
service WorkerService {
  // === Batched Operations (primary) ===

  // Get everything needed to start a turn in one call
  rpc GetTurnContext(GetTurnContextRequest) returns (GetTurnContextResponse);

  // Stream events efficiently
  rpc EmitEventStream(stream EmitEventRequest) returns (stream EmitEventResponse);

  // === Individual Operations (for specific needs) ===

  // Messages
  rpc AddMessage(AddMessageRequest) returns (AddMessageResponse);
  rpc GetMessage(GetMessageRequest) returns (GetMessageResponse);
  rpc ListMessages(ListMessagesRequest) returns (ListMessagesResponse);

  // Events (single)
  rpc EmitEvent(EmitEventRequest) returns (EmitEventResponse);

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

### GetTurnContext - Batched Read

Single call to get everything needed for a turn:

```protobuf
message GetTurnContextRequest {
  string session_id = 1;
  string agent_id = 2;
  optional string model_id = 3;  // If not set, uses agent's default
}

message GetTurnContextResponse {
  Agent agent = 1;
  Session session = 2;
  repeated Message messages = 3;
  ModelWithProvider model_provider = 4;
}
```

**Reduces:** 4 calls → 1 call at turn start

### EmitEventStream - Batched Write

Stream events instead of individual calls:

```protobuf
// Client streams events, server acknowledges each
rpc EmitEventStream(stream EmitEventRequest) returns (stream EmitEventResponse);
```

**Reduces:** N event calls → 1 streaming connection

### Call Pattern Per Turn

```
Turn Start:
  GetTurnContext()           # 1 call - get agent, session, messages, model

Turn Execution (loop):
  [LLM call - direct to provider]
  AddMessage()               # 1 call per assistant message
  [Tool execution - local]
  AddMessage()               # 1 call per tool result

Throughout:
  EmitEventStream()          # 1 streaming connection for all events
```

**Total: ~3-5 gRPC calls per turn** (down from 10+)

## Type System

### Rust Schemas (Source of Truth)

```rust
// crates/schemas/src/message.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub struct Message {
    pub id: Uuid,
    pub role: MessageRole,
    pub content: Vec<ContentPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,
    pub created_at: DateTime<Utc>,
}
```

### Proto (Mirrors Rust)

```protobuf
// crates/internal-protocol/proto/types.proto

message Message {
  string id = 1;
  MessageRole role = 2;
  repeated ContentPart content = 3;
  optional string controls_json = 4;
  google.protobuf.Timestamp created_at = 5;
}
```

### Conversion Layer

```rust
// crates/internal-protocol/src/convert.rs

impl From<schemas::Message> for proto::Message {
    fn from(m: schemas::Message) -> Self {
        proto::Message {
            id: m.id.to_string(),
            role: m.role.into(),
            content: m.content.into_iter().map(Into::into).collect(),
            controls_json: m.controls.map(|c| serde_json::to_string(&c).unwrap()),
            created_at: Some(m.created_at.into()),
        }
    }
}

impl TryFrom<proto::Message> for schemas::Message {
    type Error = ConversionError;

    fn try_from(m: proto::Message) -> Result<Self, Self::Error> {
        Ok(schemas::Message {
            id: m.id.parse()?,
            role: m.role.try_into()?,
            content: m.content.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
            controls: m.controls_json.map(|j| serde_json::from_str(&j)).transpose()?,
            created_at: m.created_at.ok_or(ConversionError::MissingField("created_at"))?.into(),
        })
    }
}
```

## Implementation Plan

Single migration, no backward compatibility. Execute in order:

### Step 1: Create `schemas` crate

```bash
crates/schemas/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── agent.rs
    ├── session.rs
    ├── message.rs
    ├── events.rs
    ├── tools.rs
    ├── llm.rs
    └── files.rs
```

- [ ] Create crate structure
- [ ] Move types from `everruns-core`
- [ ] Add `openapi` feature with utoipa
- [ ] Update all dependents to use `schemas`

### Step 2: Create `runtime` crate

```bash
crates/runtime/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── atoms/
    │   ├── mod.rs
    │   ├── input.rs
    │   ├── reason.rs
    │   └── act.rs
    ├── capabilities/
    │   ├── mod.rs
    │   ├── file_system.rs
    │   ├── web_fetch.rs
    │   └── ...
    ├── drivers.rs
    ├── tools.rs
    └── traits.rs
```

- [ ] Create crate structure
- [ ] Move atoms from `everruns-core`
- [ ] Move capabilities from `everruns-core`
- [ ] Move tool execution from `everruns-core`
- [ ] Move driver registry integration
- [ ] Define store traits

### Step 3: Create `internal-protocol` crate

```bash
crates/internal-protocol/
├── Cargo.toml
├── build.rs
├── proto/
│   ├── worker_service.proto
│   └── types.proto
└── src/
    ├── lib.rs
    └── convert.rs
```

- [ ] Create crate with tonic-build
- [ ] Define WorkerService proto
- [ ] Define type protos (mirroring schemas)
- [ ] Implement conversion traits

### Step 4: Create `control-plane` crate

```bash
crates/control-plane/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── main.rs           # Binary entry point
    ├── api/              # REST (from everruns-api)
    ├── grpc/             # NEW: WorkerService implementation
    │   ├── mod.rs
    │   └── worker_service.rs
    ├── services/         # Business logic (from everruns-api)
    └── storage/          # Database (from everruns-storage)
        ├── mod.rs
        ├── repositories.rs
        ├── models.rs
        └── migrations/
```

- [ ] Rename `everruns-api` → `control-plane`
- [ ] Merge `everruns-storage` into `control-plane/storage`
- [ ] Implement `WorkerService` gRPC server
- [ ] Start both REST and gRPC servers in main.rs
- [ ] Test with grpcurl

### Step 5: Update `worker` crate

```bash
crates/worker/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── main.rs
    ├── workflow.rs
    ├── activities.rs
    ├── grpc_client.rs    # NEW
    └── stores/           # NEW: gRPC-backed stores
        ├── mod.rs
        ├── message_store.rs
        ├── agent_store.rs
        ├── session_store.rs
        ├── event_emitter.rs
        ├── llm_provider_store.rs
        └── session_file_store.rs
```

- [ ] Add gRPC client setup
- [ ] Implement `GrpcMessageStore`
- [ ] Implement `GrpcAgentStore`
- [ ] Implement `GrpcSessionStore`
- [ ] Implement `GrpcEventEmitter`
- [ ] Implement `GrpcLlmProviderStore`
- [ ] Implement `GrpcSessionFileStore`
- [ ] Remove sqlx/database dependencies
- [ ] Update activities to use gRPC stores

### Step 6: Cleanup

- [ ] Delete `crates/everruns-core/`
- [ ] Delete `crates/everruns-storage/`
- [ ] Rename `crates/everruns-worker/` → `crates/worker/`
- [ ] Rename `crates/everruns-openai/` → `crates/openai/`
- [ ] Rename `crates/everruns-anthropic/` → `crates/anthropic/`
- [ ] Update workspace Cargo.toml
- [ ] Update CLAUDE.md
- [ ] Update CI workflows
- [ ] Run full smoke test

## Configuration

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
# Removed
DATABASE_URL=...              # Not needed
ENCRYPTION_KEY=...            # Not needed

# New
CONTROL_PLANE_GRPC_URL=http://control-plane:50051

# Unchanged
TEMPORAL_ADDRESS=localhost:7233
TEMPORAL_NAMESPACE=default
TEMPORAL_TASK_QUEUE=everruns-agent-runs
```

## Idempotency and Retry Handling

Temporal retries failed activities. To handle partial writes from crashed attempts:

### exec_id Strategy

1. **exec_id = UUID v7** - time-ordered, monotonically increasing
2. **All writes tagged with exec_id** - messages, events
3. **CommitExec on success** - marks exec_id as valid
4. **Reads filter to committed exec_ids** - orphaned data ignored

### Flow

```
Attempt 1 (exec_id = "019abc..."):
  AddMessage(exec_id="019abc...")     ✓ stored
  EmitEvent(exec_id="019abc...")      ✓ stored
  [crash before commit]               ✗ no commit

Attempt 2 (exec_id = "019def..."):    # newer UUID v7
  AddMessage(exec_id="019def...")     ✓ stored
  EmitEvent(exec_id="019def...")      ✓ stored
  CommitExec(exec_id="019def...")     ✓ committed
  [activity completes]
```

### gRPC Addition

```protobuf
service WorkerService {
  // ... existing RPCs ...

  // Commit an exec_id, marking all its writes as valid
  rpc CommitExec(CommitExecRequest) returns (CommitExecResponse);
}

message CommitExecRequest {
  string session_id = 1;
  string turn_id = 2;
  string exec_id = 3;  // UUID v7
}
```

### Read Filtering

```sql
-- Messages: only from committed exec_ids
SELECT m.* FROM messages m
JOIN committed_execs c ON m.exec_id = c.exec_id
WHERE m.session_id = ?

-- If edge case of multiple committed exec_ids for same turn/atom,
-- UUID v7 ordering means latest wins automatically
```

### Benefits

- **No duplicates**: Uncommitted writes ignored on read
- **No cleanup needed**: Orphaned data filtered out (lazy cleanup optional)
- **Idempotent**: Same exec_id = same logical operation
- **Simple**: Single commit call at end of each atom

## Success Criteria

1. **Worker has no DB access**: No sqlx in dependencies
2. **Single control-plane binary**: Serves REST (:9000) + gRPC (:50051)
3. **Smoke tests pass**: Full workflow over gRPC
4. **Clean crate structure**: 7 crates with clear responsibilities
5. **Latency acceptable**: < 100ms added per turn
