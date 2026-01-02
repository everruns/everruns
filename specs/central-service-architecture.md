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

## Next Steps

1. [ ] Design internal API routes in detail
2. [ ] Implement HTTP trait implementations with retry logic
3. [ ] Add feature flags for store selection
4. [ ] Add metrics for comparing implementations
5. [ ] Test with realistic workloads
6. [ ] Document deployment configurations
