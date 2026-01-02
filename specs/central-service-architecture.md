# Central Service Architecture Investigation

## Abstract

This document investigates an alternative architecture where workers communicate with a central service (everruns API) instead of directly accessing the database. This would replace the current trait-based database adapters with HTTP-based implementations.

## Current Architecture

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
                        │            │
                        └────────────┘
```

**Current data flow:** Workers directly access PostgreSQL through trait-based abstractions:
- `DbAgentStore` → `agents` table
- `DbSessionStore` → `sessions` table
- `DbMessageStore` → `events` table (messages as events)
- `DbEventEmitter` → `events` table
- `DbLlmProviderStore` → `llm_providers` table
- `DbSessionFileStore` → `session_files` table

## Proposed Architecture

```
┌───────────────────────────────────────────────────────────────┐
│                     Central Service (API)                      │
│                                                                │
│  ┌──────────────┐  ┌────────────────┐  ┌───────────────────┐  │
│  │ Public API   │  │ Internal API   │  │ Services/Repos    │  │
│  │ /v1/*        │  │ /internal/*    │  │                   │  │
│  └──────────────┘  └────────────────┘  └───────────────────┘  │
│                            │                     │             │
│                            └─────────────────────┘             │
│                                      │                         │
│                            ┌─────────────────┐                 │
│                            │   PostgreSQL    │                 │
│                            └─────────────────┘                 │
└───────────────────────────────────────────────────────────────┘
         ▲                                       ▲
         │ Temporal                              │ Internal API
         │                                       │
┌────────┴────────┐                    ┌─────────┴────────────┐
│ Temporal Server │                    │   Worker Process     │
│                 │◄───────────────────│                      │
│ (workflow mgmt) │                    │ HttpAgentStore       │
└─────────────────┘                    │ HttpSessionStore     │
                                       │ HttpMessageStore     │
                                       │ HttpEventEmitter     │
                                       │ HttpLlmProviderStore │
                                       │ HttpSessionFileStore │
                                       │                      │
                                       │ (NO database access) │
                                       └──────────────────────┘
```

## Worker Database Operations Inventory

Analysis of `crates/everruns-worker/src/activities.rs` reveals workers need:

### 1. Agent Operations
| Operation | Trait Method | Current Implementation |
|-----------|--------------|------------------------|
| Get agent config | `AgentStore::get_agent(agent_id)` | `DbAgentStore` |

### 2. Session Operations
| Operation | Trait Method | Current Implementation |
|-----------|--------------|------------------------|
| Get session config | `SessionStore::get_session(session_id)` | `DbSessionStore` |

### 3. Message Operations
| Operation | Trait Method | Current Implementation |
|-----------|--------------|------------------------|
| Add message | `MessageStore::add(session_id, input)` | `DbMessageStore` |
| Get message | `MessageStore::get(session_id, message_id)` | `DbMessageStore` |
| Store message | `MessageStore::store(session_id, message)` | `DbMessageStore` |
| Load all messages | `MessageStore::load(session_id)` | `DbMessageStore` |
| Count messages | `MessageStore::count(session_id)` | `DbMessageStore` |

### 4. Event Operations
| Operation | Trait Method | Current Implementation |
|-----------|--------------|------------------------|
| Emit event | `EventEmitter::emit(event)` | `DbEventEmitter` |

### 5. LLM Provider Operations
| Operation | Trait Method | Current Implementation |
|-----------|--------------|------------------------|
| Get model with provider | `LlmProviderStore::get_model_with_provider(model_id)` | `DbLlmProviderStore` |
| Get default model | `LlmProviderStore::get_default_model()` | `DbLlmProviderStore` |

### 6. File Operations (for filesystem capabilities)
| Operation | Trait Method | Current Implementation |
|-----------|--------------|------------------------|
| Read file | `SessionFileStore::read_file(session_id, path)` | `DbSessionFileStore` |
| Write file | `SessionFileStore::write_file(session_id, path, content, encoding)` | `DbSessionFileStore` |
| Delete file | `SessionFileStore::delete_file(session_id, path, recursive)` | `DbSessionFileStore` |
| List directory | `SessionFileStore::list_directory(session_id, path)` | `DbSessionFileStore` |
| Stat file | `SessionFileStore::stat_file(session_id, path)` | `DbSessionFileStore` |
| Grep files | `SessionFileStore::grep_files(session_id, pattern, path_pattern)` | `DbSessionFileStore` |
| Create directory | `SessionFileStore::create_directory(session_id, path)` | `DbSessionFileStore` |

## Required Internal API Endpoints

### Agent Endpoints
```
GET /internal/agents/{agent_id}
    Response: Agent (with capabilities resolved)
```

### Session Endpoints
```
GET /internal/sessions/{session_id}
    Response: Session
```

### Message Endpoints
```
POST /internal/sessions/{session_id}/messages
    Body: InputMessage
    Response: Message

GET /internal/sessions/{session_id}/messages/{message_id}
    Response: Message

GET /internal/sessions/{session_id}/messages
    Query: ?offset=0&limit=100
    Response: Message[]

GET /internal/sessions/{session_id}/messages/count
    Response: { count: number }
```

### Event Endpoints
```
POST /internal/sessions/{session_id}/events
    Body: Event
    Response: { sequence: number }
```

### LLM Provider Endpoints
```
GET /internal/models/{model_id}/provider
    Response: ModelWithProvider (includes decrypted API key)

GET /internal/models/default
    Response: ModelWithProvider
```

### File Endpoints
```
GET /internal/sessions/{session_id}/files?path={path}
    Response: SessionFile

PUT /internal/sessions/{session_id}/files
    Body: { path, content, encoding }
    Response: SessionFile

DELETE /internal/sessions/{session_id}/files?path={path}&recursive={bool}
    Response: { deleted: bool }

GET /internal/sessions/{session_id}/files/list?path={path}
    Response: FileInfo[]

GET /internal/sessions/{session_id}/files/stat?path={path}
    Response: FileStat

GET /internal/sessions/{session_id}/files/grep?pattern={pattern}&path_pattern={pattern}
    Response: GrepMatch[]

POST /internal/sessions/{session_id}/files/mkdir
    Body: { path }
    Response: FileInfo
```

## Advantages

### 1. Security Isolation
- Workers don't need database credentials
- Database is only accessible from the central service
- Workers can run in less trusted environments (edge, customer premises)
- Encryption keys stay centralized

### 2. Operational Simplicity
- Single point of database connection management
- Easier connection pool management
- Centralized schema migrations
- Workers become truly stateless

### 3. Scalability
- Workers can scale independently without database connection limits
- Can add caching layer in central service
- Can add rate limiting per worker
- Easier to add read replicas (workers don't need to know)

### 4. Multi-tenancy
- Central service can enforce tenant isolation
- Workers don't need tenant context
- Easier audit logging

### 5. API Key Management
- Decrypted API keys never leave the central service
- Workers receive provider configs without sensitive data
- LLM calls could be proxied through central service

### 6. Deployment Flexibility
- Workers can be deployed anywhere with network access to API
- No need to distribute database credentials
- Easier to run workers in Kubernetes, serverless, etc.

## Disadvantages

### 1. Increased Latency
- Every DB operation becomes an HTTP roundtrip
- Typical activity might make 5-10 DB operations
- At ~1-5ms per call, adds 5-50ms per activity
- Could be significant for tool-heavy workloads

### 2. Reduced Reliability
- Network failures between worker and API
- API becomes a single point of failure
- Need retry logic for transient failures
- Circuit breaker patterns needed

### 3. Increased Complexity
- More code to maintain (HTTP clients, internal API)
- More things that can go wrong
- Harder to debug (distributed tracing needed)
- Need to handle HTTP errors, timeouts, retries

### 4. Resource Overhead
- HTTP serialization/deserialization overhead
- TLS overhead if using HTTPS internally
- More CPU for JSON parsing
- Higher memory for buffering

### 5. Consistency Challenges
- Harder to maintain transactional consistency
- Events might be emitted but not acknowledged
- Need idempotency keys for retries
- Potential for duplicate events

### 6. Authentication Complexity
- Workers need API tokens
- Token rotation and management
- Per-worker or shared tokens?

## Implementation Considerations

### Authentication Options

**Option A: Shared Secret**
```rust
// Worker configuration
EVERRUNS_INTERNAL_API_URL=http://api:9000/internal
EVERRUNS_INTERNAL_API_KEY=shared-secret-key
```
- Simple but less secure
- All workers share same key
- No per-worker audit trail

**Option B: Per-Worker Tokens**
```rust
// Each worker has unique token
EVERRUNS_WORKER_ID=worker-1
EVERRUNS_WORKER_TOKEN=jwt-token-for-worker-1
```
- Better audit trail
- Can revoke individual workers
- More operational complexity

**Option C: mTLS**
```rust
// Mutual TLS between workers and API
EVERRUNS_INTERNAL_CA_CERT=/path/to/ca.crt
EVERRUNS_WORKER_CERT=/path/to/worker.crt
EVERRUNS_WORKER_KEY=/path/to/worker.key
```
- Strongest security
- Certificate management overhead
- Good for zero-trust environments

### HTTP Client Implementation

```rust
// Example HttpMessageStore implementation
pub struct HttpMessageStore {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

#[async_trait]
impl MessageStore for HttpMessageStore {
    async fn add(&self, session_id: Uuid, input: InputMessage) -> Result<Message> {
        let response = self.client
            .post(format!("{}/internal/sessions/{}/messages", self.base_url, session_id))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&input)
            .send()
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        if !response.status().is_success() {
            return Err(AgentLoopError::store(format!(
                "API error: {}",
                response.status()
            )));
        }

        response.json().await
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }

    // ... other methods
}
```

### Caching Strategy

To mitigate latency, implement caching for read-heavy operations:

```rust
pub struct CachedHttpAgentStore {
    http_store: HttpAgentStore,
    cache: moka::future::Cache<Uuid, Agent>,
}

impl CachedHttpAgentStore {
    pub fn new(http_store: HttpAgentStore) -> Self {
        let cache = moka::future::Cache::builder()
            .time_to_live(Duration::from_secs(60))
            .max_capacity(1000)
            .build();
        Self { http_store, cache }
    }
}

#[async_trait]
impl AgentStore for CachedHttpAgentStore {
    async fn get_agent(&self, agent_id: Uuid) -> Result<Option<Agent>> {
        if let Some(agent) = self.cache.get(&agent_id).await {
            return Ok(Some(agent));
        }

        let agent = self.http_store.get_agent(agent_id).await?;
        if let Some(ref a) = agent {
            self.cache.insert(agent_id, a.clone()).await;
        }
        Ok(agent)
    }
}
```

### Retry Strategy

```rust
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(5),
            backoff_multiplier: 2.0,
        }
    }
}
```

### Batching Optimization

For operations like loading messages, batch requests:

```rust
// Instead of multiple individual requests
for msg_id in message_ids {
    let msg = store.get(session_id, msg_id).await?;
}

// Use batch endpoint
POST /internal/sessions/{session_id}/messages/batch
Body: { ids: [uuid1, uuid2, ...] }
Response: Message[]
```

## Migration Path

### Phase 1: Add Internal API (Non-breaking)
1. Add internal API routes to `everruns-api`
2. Add HTTP trait implementations to `everruns-worker`
3. Feature flag: `EVERRUNS_USE_HTTP_STORES=false`

### Phase 2: Parallel Testing
1. Run workers with both implementations
2. Compare results and latency
3. Monitor for discrepancies

### Phase 3: Gradual Rollout
1. Enable HTTP stores for new deployments
2. Monitor latency and error rates
3. Roll back if issues arise

### Phase 4: Remove Database Access
1. Remove database dependency from worker crate
2. Update deployment configurations
3. Update documentation

## Decision Criteria

### When Central Service Makes Sense
- ✅ Multi-tenant SaaS with security requirements
- ✅ Workers deployed in untrusted environments
- ✅ Need strict API key isolation
- ✅ Already have low-latency internal network
- ✅ Database connection limits are a concern

### When Direct Database Makes Sense
- ✅ Single-tenant or self-hosted deployments
- ✅ Workers run alongside API (same VPC/cluster)
- ✅ Low latency is critical
- ✅ Simpler operational model preferred
- ✅ Tight consistency requirements

## Hybrid Approach

Consider a hybrid where sensitive operations go through API while high-volume operations use direct DB:

```
┌─────────────────────────────────────────────────────┐
│                    Worker Process                    │
│                                                      │
│  ┌──────────────────┐  ┌──────────────────────────┐ │
│  │ HttpLlmProvider  │  │ DbMessageStore           │ │
│  │ HttpAgentStore   │  │ DbEventEmitter           │ │
│  │ (sensitive data) │  │ (high-volume operations) │ │
│  └────────┬─────────┘  └───────────┬──────────────┘ │
│           │                        │                 │
└───────────┼────────────────────────┼─────────────────┘
            │                        │
            ▼                        ▼
     ┌──────────────┐       ┌────────────────┐
     │ Central API  │       │   PostgreSQL   │
     │ (via HTTP)   │       │   (direct)     │
     └──────────────┘       └────────────────┘
```

This gives:
- Security for API keys (HTTP)
- Performance for messages/events (direct DB)
- Flexibility to adjust based on requirements

## Recommendation

For initial investigation, recommend starting with a **feature-flagged hybrid approach**:

1. Implement HTTP stores for sensitive data (LLM providers, agent configs)
2. Keep direct DB for high-volume operations (messages, events)
3. Add metrics to compare latency and reliability
4. Make the choice configurable per deployment

This provides:
- Security benefits where they matter most
- Performance where it matters most
- Flexibility to adjust based on real-world data
- Non-breaking migration path

## RPC-Based Communication (Alternative to REST)

Instead of REST/HTTP, worker-to-central-service communication could use RPC. The most viable option for Rust is **gRPC with tonic**.

### Why gRPC?

| Aspect | REST/JSON | gRPC/Protobuf |
|--------|-----------|---------------|
| Serialization | JSON (text, ~2-10x larger) | Protobuf (binary, compact) |
| Transport | HTTP/1.1 (one request per connection) | HTTP/2 (multiplexed streams) |
| Type Safety | Runtime validation | Compile-time generated types |
| Streaming | Requires SSE/WebSocket | Native bidirectional streaming |
| Latency | Higher (text parsing, no multiplexing) | Lower (~30-50% less overhead) |
| Ecosystem | Universal | Strong in Rust (tonic), Go, Java |
| Debugging | Easy (curl, browser) | Harder (need grpcurl, special tools) |

### Industry Precedent

Many distributed systems use gRPC for worker/control-plane communication:

| System | Use Case | Communication Pattern |
|--------|----------|----------------------|
| **Temporal** | Workflow orchestration | Workers long-poll server, report completions |
| **Kubernetes** | Container orchestration | kubelet ↔ CRI, kubelet ↔ API server |
| **Ray** | Distributed computing | Worker nodes ↔ head node |
| **Envoy/Istio** | Service mesh | xDS protocol for config distribution |
| **etcd** | Distributed KV store | All client-server communication |
| **CockroachDB** | Distributed SQL | Inter-node communication |
| **Vitess** | MySQL scaling | VTGate ↔ VTTablet |

**Why these systems chose gRPC:**
- **Temporal**: "The only requirement is to be able to communicate over gRPC with a Temporal Cluster" - enables cross-language workers (Go, PHP, Rust, etc.)
- **Kubernetes**: CRI over gRPC performs 7-10x faster than REST for container operations
- **Ray**: Added gRPC proxy for "significantly lower communication latency than HTTP"

### Relevance to Everruns

Temporal already uses gRPC internally. Workers already have gRPC dependencies through the Temporal SDK (`temporal-sdk-core`, `tonic`, `prost`). Adding a gRPC service for worker communication:
- Reuses existing dependencies (no new dep overhead)
- Aligns with proven distributed systems patterns
- Enables future cross-language workers if needed

### Service Definition

```protobuf
// proto/worker_service.proto
syntax = "proto3";
package everruns.worker.v1;

import "google/protobuf/timestamp.proto";

// ============================================================================
// Worker Service - Central service for worker operations
// ============================================================================

service WorkerService {
  // Agent operations
  rpc GetAgent(GetAgentRequest) returns (GetAgentResponse);

  // Session operations
  rpc GetSession(GetSessionRequest) returns (GetSessionResponse);

  // Message operations
  rpc AddMessage(AddMessageRequest) returns (AddMessageResponse);
  rpc GetMessage(GetMessageRequest) returns (GetMessageResponse);
  rpc ListMessages(ListMessagesRequest) returns (ListMessagesResponse);
  rpc CountMessages(CountMessagesRequest) returns (CountMessagesResponse);

  // Event operations - with streaming option
  rpc EmitEvent(EmitEventRequest) returns (EmitEventResponse);
  rpc EmitEventStream(stream EmitEventRequest) returns (stream EmitEventResponse);

  // LLM Provider operations
  rpc GetModelProvider(GetModelProviderRequest) returns (GetModelProviderResponse);
  rpc GetDefaultModel(GetDefaultModelRequest) returns (GetModelProviderResponse);

  // File operations
  rpc ReadFile(ReadFileRequest) returns (ReadFileResponse);
  rpc WriteFile(WriteFileRequest) returns (WriteFileResponse);
  rpc DeleteFile(DeleteFileRequest) returns (DeleteFileResponse);
  rpc ListDirectory(ListDirectoryRequest) returns (ListDirectoryResponse);
  rpc StatFile(StatFileRequest) returns (StatFileResponse);
  rpc GrepFiles(GrepFilesRequest) returns (GrepFilesResponse);
  rpc CreateDirectory(CreateDirectoryRequest) returns (CreateDirectoryResponse);
}

// ============================================================================
// Common Types
// ============================================================================

message Uuid {
  string value = 1;
}

message ContentPart {
  oneof content {
    TextContent text = 1;
    ImageContent image = 2;
    ToolCallContent tool_call = 3;
    ToolResultContent tool_result = 4;
  }
}

message TextContent {
  string text = 1;
}

message ImageContent {
  string url = 1;
  optional string media_type = 2;
}

message ToolCallContent {
  string id = 1;
  string name = 2;
  string arguments_json = 3;  // JSON-encoded arguments
}

message ToolResultContent {
  string tool_call_id = 1;
  optional string result_json = 2;
  optional string error = 3;
}

// ============================================================================
// Agent Messages
// ============================================================================

message GetAgentRequest {
  Uuid agent_id = 1;
}

message GetAgentResponse {
  optional Agent agent = 1;
}

message Agent {
  Uuid id = 1;
  string name = 2;
  optional string system_prompt = 3;
  optional Uuid default_model_id = 4;
  repeated string capabilities = 5;
  repeated ToolDefinition tools = 6;
  google.protobuf.Timestamp created_at = 7;
}

message ToolDefinition {
  string name = 1;
  string description = 2;
  string parameters_json = 3;  // JSON Schema
  ToolPolicy policy = 4;
}

enum ToolPolicy {
  TOOL_POLICY_UNSPECIFIED = 0;
  TOOL_POLICY_AUTO = 1;
  TOOL_POLICY_CONFIRM = 2;
  TOOL_POLICY_DISABLED = 3;
}

// ============================================================================
// Session Messages
// ============================================================================

message GetSessionRequest {
  Uuid session_id = 1;
}

message GetSessionResponse {
  optional Session session = 1;
}

message Session {
  Uuid id = 1;
  Uuid agent_id = 2;
  SessionStatus status = 3;
  optional string workflow_id = 4;
  google.protobuf.Timestamp created_at = 5;
}

enum SessionStatus {
  SESSION_STATUS_UNSPECIFIED = 0;
  SESSION_STATUS_PENDING = 1;
  SESSION_STATUS_RUNNING = 2;
  SESSION_STATUS_COMPLETED = 3;
  SESSION_STATUS_FAILED = 4;
}

// ============================================================================
// Message Messages
// ============================================================================

message AddMessageRequest {
  Uuid session_id = 1;
  InputMessage input = 2;
}

message InputMessage {
  MessageRole role = 1;
  repeated ContentPart content = 2;
  optional string controls_json = 3;
  optional string metadata_json = 4;
}

enum MessageRole {
  MESSAGE_ROLE_UNSPECIFIED = 0;
  MESSAGE_ROLE_USER = 1;
  MESSAGE_ROLE_ASSISTANT = 2;
  MESSAGE_ROLE_SYSTEM = 3;
  MESSAGE_ROLE_TOOL_RESULT = 4;
}

message AddMessageResponse {
  Message message = 1;
}

message GetMessageRequest {
  Uuid session_id = 1;
  Uuid message_id = 2;
}

message GetMessageResponse {
  optional Message message = 1;
}

message ListMessagesRequest {
  Uuid session_id = 1;
  uint32 offset = 2;
  uint32 limit = 3;
}

message ListMessagesResponse {
  repeated Message messages = 1;
}

message CountMessagesRequest {
  Uuid session_id = 1;
}

message CountMessagesResponse {
  uint32 count = 1;
}

message Message {
  Uuid id = 1;
  MessageRole role = 2;
  repeated ContentPart content = 3;
  optional string controls_json = 4;
  optional string metadata_json = 5;
  google.protobuf.Timestamp created_at = 6;
}

// ============================================================================
// Event Messages
// ============================================================================

message EmitEventRequest {
  Event event = 1;
}

message EmitEventResponse {
  int32 sequence = 1;
}

message Event {
  Uuid id = 1;
  Uuid session_id = 2;
  string event_type = 3;
  EventContext context = 4;
  string data_json = 5;  // Typed event data as JSON
  google.protobuf.Timestamp timestamp = 6;
}

message EventContext {
  optional Uuid turn_id = 1;
  optional Uuid exec_id = 2;
  optional Uuid input_message_id = 3;
}

// ============================================================================
// LLM Provider Messages
// ============================================================================

message GetModelProviderRequest {
  Uuid model_id = 1;
}

message GetDefaultModelRequest {}

message GetModelProviderResponse {
  optional ModelWithProvider model = 1;
}

message ModelWithProvider {
  string model = 1;
  LlmProviderType provider_type = 2;
  optional string api_key = 3;  // Decrypted
  optional string base_url = 4;
}

enum LlmProviderType {
  LLM_PROVIDER_TYPE_UNSPECIFIED = 0;
  LLM_PROVIDER_TYPE_OPENAI = 1;
  LLM_PROVIDER_TYPE_ANTHROPIC = 2;
  LLM_PROVIDER_TYPE_AZURE = 3;
}

// ============================================================================
// File Operation Messages
// ============================================================================

message ReadFileRequest {
  Uuid session_id = 1;
  string path = 2;
}

message ReadFileResponse {
  optional SessionFile file = 1;
}

message SessionFile {
  string path = 1;
  string content = 2;
  string encoding = 3;
  google.protobuf.Timestamp created_at = 4;
  google.protobuf.Timestamp updated_at = 5;
}

message WriteFileRequest {
  Uuid session_id = 1;
  string path = 2;
  string content = 3;
  string encoding = 4;
}

message WriteFileResponse {
  SessionFile file = 1;
}

message DeleteFileRequest {
  Uuid session_id = 1;
  string path = 2;
  bool recursive = 3;
}

message DeleteFileResponse {
  bool deleted = 1;
}

message ListDirectoryRequest {
  Uuid session_id = 1;
  string path = 2;
}

message ListDirectoryResponse {
  repeated FileInfo files = 1;
}

message FileInfo {
  string name = 1;
  string path = 2;
  bool is_directory = 3;
  uint64 size = 4;
}

message StatFileRequest {
  Uuid session_id = 1;
  string path = 2;
}

message StatFileResponse {
  optional FileStat stat = 1;
}

message FileStat {
  string path = 1;
  bool exists = 2;
  bool is_directory = 3;
  uint64 size = 4;
  google.protobuf.Timestamp created_at = 5;
  google.protobuf.Timestamp modified_at = 6;
}

message GrepFilesRequest {
  Uuid session_id = 1;
  string pattern = 2;
  optional string path_pattern = 3;
}

message GrepFilesResponse {
  repeated GrepMatch matches = 1;
}

message GrepMatch {
  string path = 1;
  uint32 line_number = 2;
  string line = 3;
}

message CreateDirectoryRequest {
  Uuid session_id = 1;
  string path = 2;
}

message CreateDirectoryResponse {
  FileInfo directory = 1;
}
```

### Rust Implementation with Tonic

**Server side (in everruns-api):**

```rust
// crates/everruns-api/src/grpc/worker_service.rs
use tonic::{Request, Response, Status};
use crate::proto::worker_service_server::WorkerService;
use crate::proto::*;

pub struct WorkerServiceImpl {
    db: Database,
    encryption: EncryptionService,
}

#[tonic::async_trait]
impl WorkerService for WorkerServiceImpl {
    async fn get_agent(
        &self,
        request: Request<GetAgentRequest>,
    ) -> Result<Response<GetAgentResponse>, Status> {
        let agent_id = parse_uuid(&request.get_ref().agent_id)?;

        let agent = self.db
            .get_agent(agent_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get agent: {}", e);
                Status::internal("Internal error")
            })?;

        Ok(Response::new(GetAgentResponse {
            agent: agent.map(Into::into),
        }))
    }

    async fn emit_event(
        &self,
        request: Request<EmitEventRequest>,
    ) -> Result<Response<EmitEventResponse>, Status> {
        let event = request.into_inner().event
            .ok_or_else(|| Status::invalid_argument("event required"))?;

        let sequence = self.db
            .create_event(event.try_into()?)
            .await
            .map_err(|e| {
                tracing::error!("Failed to emit event: {}", e);
                Status::internal("Internal error")
            })?;

        Ok(Response::new(EmitEventResponse { sequence }))
    }

    // Streaming variant for high-throughput event emission
    async fn emit_event_stream(
        &self,
        request: Request<tonic::Streaming<EmitEventRequest>>,
    ) -> Result<Response<Self::EmitEventStreamStream>, Status> {
        let mut stream = request.into_inner();
        let db = self.db.clone();

        let output = async_stream::stream! {
            while let Some(req) = stream.next().await {
                match req {
                    Ok(req) => {
                        if let Some(event) = req.event {
                            match db.create_event(event.try_into().unwrap()).await {
                                Ok(seq) => yield Ok(EmitEventResponse { sequence: seq }),
                                Err(e) => yield Err(Status::internal(e.to_string())),
                            }
                        }
                    }
                    Err(e) => yield Err(e),
                }
            }
        };

        Ok(Response::new(Box::pin(output)))
    }
}
```

**Client side (in everruns-worker):**

```rust
// crates/everruns-worker/src/grpc_adapters.rs
use tonic::transport::Channel;
use crate::proto::worker_service_client::WorkerServiceClient;

pub struct GrpcMessageStore {
    client: WorkerServiceClient<Channel>,
}

impl GrpcMessageStore {
    pub async fn connect(endpoint: &str) -> Result<Self> {
        let channel = Channel::from_shared(endpoint.to_string())?
            .connect()
            .await?;
        Ok(Self {
            client: WorkerServiceClient::new(channel),
        })
    }
}

#[async_trait]
impl MessageStore for GrpcMessageStore {
    async fn add(&self, session_id: Uuid, input: InputMessage) -> Result<Message> {
        let mut client = self.client.clone();

        let response = client
            .add_message(AddMessageRequest {
                session_id: Some(session_id.into()),
                input: Some(input.into()),
            })
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        response
            .into_inner()
            .message
            .ok_or_else(|| AgentLoopError::store("No message in response"))?
            .try_into()
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let mut client = self.client.clone();

        let response = client
            .list_messages(ListMessagesRequest {
                session_id: Some(session_id.into()),
                offset: 0,
                limit: 10000,  // Large limit to get all
            })
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        response
            .into_inner()
            .messages
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }
}
```

### gRPC vs REST Comparison for Everruns

| Operation | REST Overhead | gRPC Overhead | Savings |
|-----------|--------------|---------------|---------|
| Get Agent | ~500 bytes JSON | ~150 bytes proto | 70% |
| Emit Event | ~1KB JSON | ~300 bytes proto | 70% |
| Load 100 Messages | ~50KB JSON | ~15KB proto | 70% |
| Connection Setup | New TCP per request | Multiplexed streams | 90%+ |

**Latency comparison (same network):**
- REST: ~2-5ms per request (connection + serialization)
- gRPC: ~0.5-2ms per request (reused connection, binary)

For a typical activity with 5-10 operations:
- REST: 10-50ms overhead
- gRPC: 2.5-20ms overhead

### Streaming Benefits

gRPC enables bidirectional streaming, opening new architectural possibilities:

```
┌─────────────────────────────────────────────────────────────┐
│                     Central Service                          │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │              gRPC Worker Service                      │   │
│  │                                                       │   │
│  │  Unary RPCs:        Streaming RPCs:                   │   │
│  │  - GetAgent         - EmitEventStream (client→server) │   │
│  │  - GetSession       - SubscribeEvents (server→client) │   │
│  │  - AddMessage       - BidiStream (both directions)    │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                              ▲
                              │ gRPC (HTTP/2)
                              │ Multiplexed streams
                              │ Binary protobuf
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                     Worker Process                           │
│                                                              │
│  Single persistent connection, multiple concurrent streams   │
└─────────────────────────────────────────────────────────────┘
```

**Streaming use cases:**
1. **Event batching**: Accumulate events, send in batches via stream
2. **Server push**: Central service could push config updates to workers
3. **Health monitoring**: Persistent connection doubles as health check

### Authentication with gRPC

**Option 1: Metadata (like HTTP headers)**
```rust
let mut request = Request::new(GetAgentRequest { ... });
request.metadata_mut().insert(
    "authorization",
    format!("Bearer {}", token).parse().unwrap(),
);
```

**Option 2: Interceptor (applies to all requests)**
```rust
let channel = Channel::from_shared(endpoint)?
    .connect()
    .await?;

let client = WorkerServiceClient::with_interceptor(channel, |mut req: Request<()>| {
    req.metadata_mut().insert(
        "authorization",
        "Bearer token".parse().unwrap(),
    );
    Ok(req)
});
```

**Option 3: mTLS (certificate-based)**
```rust
let tls = ClientTlsConfig::new()
    .ca_certificate(Certificate::from_pem(ca_cert))
    .identity(Identity::from_pem(client_cert, client_key));

let channel = Channel::from_shared(endpoint)?
    .tls_config(tls)?
    .connect()
    .await?;
```

### Build Integration

Add to `build.rs`:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)  // For everruns-api
        .build_client(true)  // For everruns-worker
        .compile(
            &["proto/worker_service.proto"],
            &["proto"],
        )?;
    Ok(())
}
```

### RPC Recommendation

**Prefer gRPC over REST for worker communication because:**

1. **Already in the ecosystem**: Temporal uses gRPC, dependencies already present
2. **Lower latency**: 50-70% reduction in per-operation overhead
3. **Type safety**: Proto definitions generate matching Rust types
4. **Streaming**: Enables future optimizations (event batching, server push)
5. **Connection efficiency**: Single multiplexed connection vs per-request

**When REST might still be preferred:**
- Debugging ease (curl-friendly)
- External integrations (webhooks)
- Browser-based admin tools
- Simpler deployment (no proto tooling)

### Hybrid: gRPC Internal + REST External

```
┌─────────────────────────────────────────────────────────────┐
│                     Central Service                          │
│                                                              │
│  ┌─────────────────────┐    ┌─────────────────────────────┐ │
│  │   REST API (axum)   │    │    gRPC Service (tonic)     │ │
│  │   /v1/* (public)    │    │    :50051 (internal)        │ │
│  │   /swagger-ui/      │    │    WorkerService            │ │
│  └──────────┬──────────┘    └──────────────┬──────────────┘ │
│             │                              │                 │
│             └──────────────┬───────────────┘                 │
│                            │                                 │
│                    ┌───────┴───────┐                         │
│                    │   Services    │                         │
│                    │   Database    │                         │
│                    └───────────────┘                         │
└─────────────────────────────────────────────────────────────┘
         ▲                                    ▲
         │ REST                               │ gRPC
         │                                    │
    ┌────┴────┐                      ┌────────┴────────┐
    │ Web UI  │                      │     Workers     │
    │ Clients │                      │                 │
    └─────────┘                      └─────────────────┘
```

This gives:
- gRPC performance for high-volume worker traffic
- REST simplicity for client/browser integrations
- Same backend services, different transport

## Next Steps

1. [ ] Design internal API routes in detail (REST or gRPC)
2. [ ] Implement HTTP/gRPC trait implementations with retry logic
3. [ ] Add feature flags for store selection
4. [ ] Add metrics for comparing implementations
5. [ ] Test with realistic workloads
6. [ ] Document deployment configurations
7. [ ] If gRPC: Set up proto build pipeline and code generation
