# Parallel Agent Execution at 1M Scale

## Abstract

Design for running 1,000,000 concurrent agent sessions on everruns. Identifies bottlenecks in the current single-PostgreSQL architecture and proposes incremental changes across database, worker pool, task distribution, and LLM routing layers.

## Current Architecture Summary

```
Clients → REST API (control plane) → PostgreSQL (single instance)
                                   ↕ gRPC
                              Workers (stateless)
                                   ↓
                              LLM Providers
```

- **Control plane**: Single server process, owns all state in PostgreSQL
- **Workers**: Stateless executors, communicate via gRPC, default 1000 concurrent tasks each
- **Task claiming**: `SELECT FOR UPDATE SKIP LOCKED` on `durable_task_queue`
- **Notifications**: PostgreSQL `NOTIFY` for low-latency task push
- **Events**: All session events in single `events` table
- **Sessions**: Full PostgreSQL row per session with JSONB metadata

## Bottleneck Analysis

### 1. PostgreSQL Connection Limits

**Problem**: PostgreSQL maxes at ~500-1000 connections. With 1000 workers each holding a connection pool, plus the control plane, we exceed limits.

**Current**: Workers use gRPC → control plane → PostgreSQL (no direct DB access). But the control plane itself becomes the bottleneck multiplexing 1M operations through one connection pool.

### 2. Task Queue Contention

**Problem**: `SELECT FOR UPDATE SKIP LOCKED` on `durable_task_queue` works well at 1000s of tasks. At 1M pending tasks, the partial index on `status = 'pending'` grows massive, and row-level locking creates hot pages.

**Current**: Single `durable_task_queue` table, partitioned by `activity_type` only logically (WHERE clause).

### 3. Event Write Throughput

**Problem**: Each agent turn generates 5-20 events. At 1M concurrent agents doing ~1 turn/sec peak, that's 5-20M event INSERTs/sec. Single PostgreSQL can handle ~50-100K inserts/sec with WAL.

### 4. Session State Overhead

**Problem**: Each session carries full metadata (JSONB config, filesystem state, capabilities). At 1M rows, queries against `sessions` table with JOINs become expensive.

### 5. LLM Provider Rate Limits

**Problem**: Even with pooled keys, major providers cap at ~10K-100K RPM. 1M concurrent agents each making LLM calls would need 10-100x provider capacity.

### 6. Notification Fan-out

**Problem**: PostgreSQL `NOTIFY` is single-node, single-channel. Broadcasting task availability to 1000 workers through one `LISTEN` connection doesn't scale.

## Design: Tiered Scaling

Three tiers, each independently deployable. Tier 1 alone gets to ~50K concurrent. Tier 1+2 gets to ~500K. All three reach 1M+.

### Tier 1: Partition and Pool (target: 50K concurrent)

Changes to the existing single-instance architecture that require no new infrastructure.

#### 1a. Table Partitioning

Partition hot tables by organization/tenant ID using PostgreSQL native partitioning:

```sql
-- Task queue: partition by org_id (hash, 32 partitions)
CREATE TABLE durable_task_queue (
    id UUID NOT NULL,
    org_id UUID NOT NULL,
    activity_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    ...
) PARTITION BY HASH (org_id);

CREATE TABLE durable_task_queue_p0 PARTITION OF durable_task_queue
    FOR VALUES WITH (MODULUS 32, REMAINDER 0);
-- ... through p31

-- Events: partition by session_id (hash, 64 partitions)
CREATE TABLE events (
    id UUID NOT NULL,
    session_id UUID NOT NULL,
    ...
) PARTITION BY HASH (session_id);

-- Sessions: partition by org_id (hash, 16 partitions)
CREATE TABLE sessions (
    id UUID NOT NULL,
    org_id UUID NOT NULL,
    ...
) PARTITION BY HASH (org_id);
```

**Impact**: Reduces lock contention by 32x on task claiming. Each `SELECT FOR UPDATE SKIP LOCKED` only scans one partition.

#### 1b. Worker Affinity

Assign workers to partition ranges so they only claim tasks from their partitions:

```rust
pub struct DurableWorkerConfig {
    // Existing
    pub max_concurrent_tasks: usize,
    // New: worker claims tasks only from these org partitions
    pub partition_range: Option<Range<u32>>, // e.g., 0..8 of 32
}
```

Workers include partition filter in claim query:
```sql
SELECT * FROM durable_task_queue
WHERE status = 'pending'
  AND org_id_partition(org_id) BETWEEN $1 AND $2
ORDER BY priority DESC, created_at ASC
FOR UPDATE SKIP LOCKED
LIMIT $3
```

**Impact**: Eliminates cross-worker contention entirely. Each partition has dedicated workers.

#### 1c. Lightweight Session Creation

Add a `session_lite` mode for high-volume agent spawning. Skip expensive defaults:

```rust
pub struct SessionCreateParams {
    // Existing fields...
    /// Skip filesystem, KV store, SQL DB initialization.
    /// Session gets them lazily on first access.
    pub lite: bool,
}
```

- No VFS initialization until first file write
- No KV store allocation until first key set
- No capability resolution until first tool call (cache resolved capabilities)
- Session row uses minimal JSONB (no inline config, reference agent_id instead)

**Impact**: Session creation drops from ~5ms to ~0.5ms. Enables burst-spawning 10K sessions/sec.

#### 1d. Event Batching

Batch event inserts instead of one-at-a-time:

```rust
/// Accumulate events in memory, flush every 100ms or 50 events
pub struct EventBatcher {
    buffer: Vec<Event>,
    flush_interval: Duration,    // 100ms
    flush_threshold: usize,      // 50
}
```

Use `COPY` protocol for bulk inserts (10-50x faster than individual INSERTs):

```rust
async fn flush(&mut self, pool: &PgPool) {
    let mut copy = pool.copy_in_raw("COPY events FROM STDIN WITH (FORMAT binary)").await?;
    for event in self.buffer.drain(..) {
        copy.send(event.to_binary_row()).await?;
    }
    copy.finish().await?;
}
```

**Impact**: Event write throughput goes from ~100K/sec to ~2M/sec.

### Tier 2: Distributed Task Distribution (target: 500K concurrent)

Add a message broker for task distribution, keeping PostgreSQL as source of truth.

#### 2a. NATS JetStream for Task Notifications

Replace PostgreSQL `NOTIFY` with NATS JetStream:

```
Control Plane → NATS JetStream → Workers
     ↓                              ↓
PostgreSQL (source of truth)    gRPC (state ops)
```

- Tasks still stored in PostgreSQL (durability)
- NATS handles fan-out notifications (millions/sec)
- Workers subscribe to partition-specific subjects: `tasks.partition.{0..31}`
- Acknowledgment-based: worker ACKs after claiming from PostgreSQL

```rust
// Control plane publishes task notification
nats.publish(
    format!("tasks.partition.{}", task.partition_id()),
    TaskNotification { task_id, activity_type, priority }.encode()
).await?;

// Worker subscribes to its partitions
let sub = nats.subscribe("tasks.partition.{0..7}").await?;
while let Some(msg) = sub.next().await {
    let notification: TaskNotification = decode(&msg.data);
    // Claim from PostgreSQL via gRPC
    let task = grpc_client.claim_task(notification.task_id).await?;
    if let Some(task) = task {
        self.execute(task).await;
    }
    msg.ack().await?;
}
```

**Impact**: Notification latency stays at ~4ms. Fan-out scales to 10K+ workers. No PostgreSQL LISTEN bottleneck.

#### 2b. Read Replicas for Query Load

Add PostgreSQL read replicas for non-mutating operations:

```
Writes → Primary PostgreSQL
Reads  → Read Replica Pool (2-4 replicas)
```

Route these to replicas:
- `GET /v1/sessions` (list)
- `GET /v1/sessions/{id}/events` (history)
- Durable dashboard metrics
- Agent/harness lookups

**Impact**: Read load drops ~70% from primary. Primary focuses on writes (task claiming, event inserts).

#### 2c. Connection Pooling with PgBouncer

Deploy PgBouncer in transaction mode between control plane and PostgreSQL:

```
Control Plane (1000 app connections) → PgBouncer → PostgreSQL (200 real connections)
```

**Impact**: Supports 10x more application-level connections without hitting PostgreSQL limits.

### Tier 3: Horizontal Sharding (target: 1M+ concurrent)

Shard the control plane itself for true horizontal scale.

#### 3a. Multi-Instance Control Plane

Run N control plane instances, each owning a range of organization partitions:

```
Load Balancer
  ├─ Control Plane A (orgs partition 0-7)   → PostgreSQL Shard A
  ├─ Control Plane B (orgs partition 8-15)  → PostgreSQL Shard B
  ├─ Control Plane C (orgs partition 16-23) → PostgreSQL Shard C
  └─ Control Plane D (orgs partition 24-31) → PostgreSQL Shard D
```

Routing layer (in load balancer or thin proxy):
```rust
fn route_request(org_id: Uuid) -> ControlPlaneInstance {
    let partition = hash(org_id) % 32;
    match partition {
        0..=7   => InstanceA,
        8..=15  => InstanceB,
        16..=23 => InstanceC,
        24..=31 => InstanceD,
    }
}
```

Each shard is an independent PostgreSQL instance with its own:
- Task queue partitions
- Event tables
- Session tables
- Worker pool

**Impact**: Linear horizontal scaling. Each shard handles ~250K concurrent agents. Add shards for more.

#### 3b. LLM Request Router

Centralized LLM routing layer that pools API keys and manages rate limits:

```
Workers → LLM Router → Provider APIs
              ↓
         Rate Limiter (token bucket per provider/key)
         Queue (backpressure when at limit)
         Key Pool (rotate across N API keys)
         Priority (paid > free tier, interactive > background)
```

```rust
pub struct LlmRouter {
    providers: HashMap<ProviderId, ProviderPool>,
}

pub struct ProviderPool {
    api_keys: Vec<ApiKey>,
    rate_limiter: TokenBucket,     // per-key limits
    global_limiter: TokenBucket,   // provider-wide limits
    queue: PriorityQueue<LlmRequest>,
    max_queue_depth: usize,        // backpressure
}

impl LlmRouter {
    /// Route request to least-loaded key with capacity
    async fn route(&self, req: LlmRequest) -> Result<LlmResponse> {
        let pool = &self.providers[&req.provider];

        // Backpressure: reject if queue too deep
        if pool.queue.len() >= pool.max_queue_depth {
            return Err(Error::Backpressure("LLM queue full"));
        }

        // Find key with available rate limit
        let key = pool.least_loaded_key().await?;
        pool.rate_limiter.acquire(&key).await;
        pool.global_limiter.acquire().await;

        key.client.chat_completion(req).await
    }
}
```

**Impact**: Maximizes LLM throughput across all available API keys. Prevents 429s from killing agent runs. Priority queuing ensures interactive agents aren't starved by background batch jobs.

#### 3c. Event Streaming with Kafka/Redpanda

For 1M+ agents, move event storage to a streaming platform:

```
Workers → Kafka/Redpanda (events topic, partitioned by session_id)
                ↓
         Consumer: PostgreSQL writer (batched, async)
         Consumer: SSE broadcaster (real-time)
         Consumer: Analytics pipeline
```

- Events written to Kafka first (high-throughput append)
- Async consumer batches into PostgreSQL for durability/queries
- SSE connections read from Kafka directly (no PostgreSQL polling)
- Retention policy: hot events in Kafka (7 days), cold in PostgreSQL

**Impact**: Event write throughput effectively unlimited. Decouples write path from read path.

## Capacity Planning

| Component | Per Unit | Units Needed | Total |
|-----------|----------|-------------|-------|
| Workers | 1000 tasks each | 1000 workers | 1M tasks |
| Worker memory | ~500MB per worker | 1000 | 500GB RAM |
| PostgreSQL shards | 250K sessions each | 4 shards | 1M sessions |
| DB connections | 200 per shard | 4 shards | 800 connections |
| NATS | 1M msg/sec | 3-node cluster | 3M msg/sec |
| LLM API keys | 10K RPM each | 100 keys | 1M RPM |
| Control plane instances | 250K org capacity | 4 instances | 1M |

**Estimated infrastructure cost** (cloud):
- 1000 workers (c6i.xlarge): ~$125K/mo
- 4 PostgreSQL shards (r6g.2xlarge): ~$8K/mo
- NATS cluster (m6i.xlarge × 3): ~$1K/mo
- 4 control planes (c6i.2xlarge): ~$2K/mo
- LLM API costs: dominant cost, ~$500K-5M/mo depending on model/usage

## Implementation Phases

### Phase 1: Foundation (Tier 1) — 2-3 weeks

1. Table partitioning migration (hash by org_id/session_id)
2. Worker partition affinity in `DurableWorkerConfig`
3. Lightweight session creation (`lite` mode)
4. Event batching with `COPY` protocol
5. Load test: validate 50K concurrent sessions

### Phase 2: Distribution (Tier 2) — 3-4 weeks

1. NATS JetStream integration for task notifications
2. Read replica routing in storage layer
3. PgBouncer deployment
4. Load test: validate 500K concurrent sessions

### Phase 3: Sharding (Tier 3) — 4-6 weeks

1. Org-based routing proxy
2. Multi-instance control plane with shard assignment
3. LLM request router with key pooling
4. Kafka/Redpanda for event streaming
5. Load test: validate 1M concurrent sessions

## Key Design Decisions

1. **PostgreSQL stays as source of truth** — No migration to a different database. Partitioning and sharding within PostgreSQL ecosystem.

2. **Workers remain stateless** — No architectural change to worker model. Just more of them with partition affinity.

3. **Org-based sharding key** — Natural tenant boundary. No cross-shard queries needed for normal operations. Cross-org queries (admin dashboards) go through aggregation layer.

4. **NATS over Redis/Kafka for notifications** — Lightweight, built for pub/sub fan-out, JetStream gives at-least-once delivery. Kafka reserved for event streaming where throughput matters more than latency.

5. **Incremental adoption** — Each tier is independently deployable. A single-org deployment can stay on Tier 1 forever. Multi-tenant SaaS deployments add Tier 2-3 as needed.

## Risks

| Risk | Mitigation |
|------|-----------|
| Cross-shard operations (admin queries, global search) | Aggregation API that fans out to all shards |
| Shard hotspots (one org with 500K agents) | Sub-org sharding key (org_id + session_id hash) |
| NATS as new SPOF | 3-node cluster, fallback to PostgreSQL polling |
| Migration complexity (partitioning existing tables) | pg_partman for online partition creation, backfill in background |
| LLM cost at 1M scale | Tiered models (fast/cheap for simple tasks, expensive for complex), aggressive caching of common prompts |
