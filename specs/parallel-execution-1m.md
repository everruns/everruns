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

#### 3b. SSE Streaming at Scale

The current SSE architecture has three bottlenecks that break at 1M agents:

1. **Single broadcast channel** — `EventNotificationBroadcaster` uses one `broadcast::channel(4096)`. All event notifications for all sessions flow through it. At 1M agents doing ~1 event/sec, the channel drops messages constantly (capacity 4096), forcing SSE streams to fall back to DB polling.

2. **Single PgListener** — One PostgreSQL connection listens on `event_available`. PostgreSQL NOTIFY is single-threaded on the sender side; at ~1M notifications/sec the WAL sender can't keep up.

3. **Global connection limit** — `SseConnectionTracker` caps at 10K global SSE connections per instance. Even at 10% client attachment rate, 1M agents = 100K SSE connections.

**Solution: Shard-local SSE with NATS fan-out**

```
                    ┌─────────────────────────────────────────────┐
                    │              Load Balancer                    │
                    │   Route SSE by org_id → correct shard        │
                    └──────────┬──────────┬──────────┬────────────┘
                               │          │          │
                    ┌──────────▼──┐ ┌─────▼──────┐ ┌▼───────────┐
                    │ CP Shard A  │ │ CP Shard B │ │ CP Shard C │
                    │ 25K SSE     │ │ 25K SSE    │ │ 25K SSE    │  × N shards
                    │ connections │ │ connections│ │ connections│
                    └──────┬──────┘ └─────┬──────┘ └──────┬──────┘
                           │              │               │
                    ┌──────▼──────────────▼───────────────▼──────┐
                    │              NATS JetStream                  │
                    │   Subject: events.shard.{0..N}              │
                    │   Per-shard subjects → no cross-talk        │
                    └──────┬──────────────┬───────────────┬──────┘
                           │              │               │
                    ┌──────▼──────┐ ┌─────▼──────┐ ┌─────▼──────┐
                    │ PG Shard A  │ │ PG Shard B │ │ PG Shard C │
                    │ NOTIFY →    │ │ NOTIFY →   │ │ NOTIFY →   │
                    │ local only  │ │ local only │ │ local only │
                    └─────────────┘ └────────────┘ └────────────┘
```

**How it works at each tier:**

**Tier 1 (single instance, 50K agents):**
- Partition the broadcast channel by session_id hash (16 channels instead of 1)
- Each SSE stream subscribes to the channel matching its session's partition
- Reduces per-channel throughput from 1M msg/sec to ~3K msg/sec per channel (well within 4096 capacity)

```rust
/// Partitioned event broadcaster — reduces per-channel load
pub struct PartitionedEventBroadcaster {
    /// 16 broadcast channels, selected by session_id hash
    channels: [broadcast::Sender<EventNotificationPayload>; 16],
}

impl PartitionedEventBroadcaster {
    fn channel_for(&self, session_id: Uuid) -> usize {
        (session_id.as_u128() % 16) as usize
    }

    fn publish(&self, payload: EventNotificationPayload) {
        let idx = self.channel_for(payload.session_id);
        let _ = self.channels[idx].send(payload);
    }

    fn subscribe(&self, session_id: Uuid) -> broadcast::Receiver<EventNotificationPayload> {
        let idx = self.channel_for(session_id);
        self.channels[idx].subscribe()
    }
}
```

**Tier 2 (distributed, 500K agents):**
- Replace PostgreSQL NOTIFY with NATS for event notifications
- Workers publish to `events.session.{partition}` after writing events
- Control plane SSE streams subscribe to the partition matching their session
- NATS handles fan-out at millions of messages/sec (vs PostgreSQL NOTIFY's ~50K/sec)
- PostgreSQL NOTIFY kept only as fallback when NATS is unavailable

```rust
/// NATS-backed event notifier — replaces PgListener for scale
pub struct NatsEventNotifier {
    client: async_nats::Client,
    num_partitions: u32,
}

impl NatsEventNotifier {
    /// Worker calls this after inserting events
    async fn notify(&self, session_id: Uuid) {
        let partition = (session_id.as_u128() % self.num_partitions as u128) as u32;
        let subject = format!("events.partition.{partition}");
        self.client.publish(subject, session_id.to_string().into()).await.ok();
    }

    /// SSE stream subscribes to its session's partition
    async fn subscribe(&self, session_id: Uuid) -> async_nats::Subscriber {
        let partition = (session_id.as_u128() % self.num_partitions as u128) as u32;
        let subject = format!("events.partition.{partition}");
        self.client.subscribe(subject).await.unwrap()
    }
}
```

**Tier 3 (sharded, 1M+ agents):**
- Each control plane shard handles only its org partition's SSE connections
- 4 shards × 25K SSE connections each = 100K total (enough for 1M agents at 10% client rate)
- Each shard has its own `SseConnectionTracker` with `global_max: 25_000`
- NATS subjects are per-shard: `events.shard.{shard_id}.partition.{partition_id}`
- No cross-shard SSE traffic — load balancer routes SSE connections to the correct shard by org_id
- If a client connects to the wrong shard, return `HTTP 307 Temporary Redirect` to the correct one

**SSE connection math at 1M:**

| Metric | Value |
|--------|-------|
| Total agents | 1,000,000 |
| Agents with active SSE client | ~100,000 (10%) |
| Control plane shards | 4 |
| SSE connections per shard | ~25,000 |
| Memory per SSE connection | ~8KB (tokio task + buffers) |
| Total SSE memory per shard | ~200MB |
| NATS partitions per shard | 64 |
| Events/sec per partition | ~4,000 |

**Key change: event delivery path**

Current (polling-based fallback dominates at scale):
```
Event INSERT → PG NOTIFY → PgListener → broadcast::channel(4096) → SSE stream
                                              ↓ (overflow)
                                         SSE polls DB every 100-500ms ← kills DB
```

Proposed (push-based, no polling):
```
Event INSERT → Worker publishes to NATS → SSE stream gets push notification
                                                ↓
                                         SSE reads event by ID from DB (single row fetch)
                                         OR from local cache (for hot sessions)
```

The DB is never polled in a loop. Each SSE stream gets a targeted push for its partition, reads the specific event by ID, and sends it to the client. At 1M agents, this is ~1M targeted reads/sec across 4 PostgreSQL shards = ~250K reads/sec per shard (well within PostgreSQL capacity with connection pooling).

#### 3c. LLM Request Router

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

#### 3d. Event Log as Separate Storage

The `events` table is everruns' highest-volume table. It's append-only (enforced by `prevent_event_mutation()` trigger), ordered by `(session_id, sequence)`, and accessed almost exclusively by session_id. This is a **log**, not relational data — it shares no foreign keys with anything except sessions, and the only queries are "give me events for session X since sequence Y."

PostgreSQL is the wrong tool for a high-throughput append-only log. The current schema (4 indexes, JSONB data column, UUIDv7 PK, sequence allocation via row lock) adds overhead that a log doesn't need.

**Pattern: Event log as first-class separate storage** (same as Codex)

OpenAI Codex stores the message log separately from agent state. This isn't just an optimization — it's a recognition that events have fundamentally different access patterns from relational data:

| Property | Relational state (sessions, agents) | Event log |
|---|---|---|
| Access pattern | Random read/write by ID | Append + sequential read by session |
| Volume | Low (1 row per session) | High (100s-1000s per session) |
| Mutations | Frequent (status updates) | Never (append-only) |
| Indexes needed | Many (status, org, agent, timestamps) | One (session_id + sequence) |
| Consistency | ACID required | Ordered append sufficient |
| Retention | Permanent | Tiered (hot/warm/cold) |

**Implementation: Two-Tier Event Architecture**

Today, `EventService.emit()` stores every event — including every `output.message.delta` token chunk — to PostgreSQL via `INSERT INTO events`. This is the single biggest scalability problem. Delta events are:

- **Ephemeral**: nobody reads them back. `output.message.completed` has the final text.
- **Highest volume**: a 500-token response at ~100ms batching = ~50 delta events per turn. At 1M agents, that's ~50M delta events/sec peak.
- **Latency-critical**: users expect <100ms token delivery. Kafka adds 2-5ms per message (batching, replication, consumer poll). Acceptable but unnecessary — why persist something nobody reads?

**Tier 1: Ephemeral events — in-memory pub/sub, never stored**

Delta events (`output.message.delta`, `reason.thinking.delta`, `tool.output.delta`) go through an in-process broadcast channel (current deployment) or NATS core pub/sub (multi-instance). No Kafka. No PostgreSQL. No persistence at all.

```
Worker LLM stream                     SSE client
  ├─ token chunk                        │
  ├─ token chunk    ──► NATS pub/sub ──►│ (~1ms latency)
  ├─ token chunk        (fire & forget) │
  └─ [completed]    ──► Kafka ──► PG    │ (durable)
```

```rust
/// EventService.emit() — split by event durability
pub async fn emit(&self, request: EventRequest) -> Result<Event> {
    Self::validate_event_type_consistency(&request)?;

    if request.is_ephemeral() {
        // Ephemeral: broadcast only, no storage
        // Sequence assigned from in-memory counter (per session)
        let event = self.to_ephemeral_event(request);
        self.ephemeral_broadcaster.publish(&event);
        return Ok(event);
    }

    // Durable: publish to Kafka, sequence = partition offset
    let event = self.kafka_producer.publish(request).await?;

    // Notify listeners (OTel, metrics)
    self.notify_listeners(&event).await;
    Ok(event)
}

impl EventRequest {
    fn is_ephemeral(&self) -> bool {
        matches!(self.event_type.as_str(),
            "output.message.delta"
            | "output.message.started"
            | "reason.thinking.delta"
            | "reason.thinking.started"
            | "tool.output.delta"
        )
    }
}
```

**Why not Kafka for deltas:**
- Kafka's minimum end-to-end latency is ~2-5ms (producer batch → broker → consumer poll). Acceptable for durable events. Wasteful for deltas that are consumed in real-time and thrown away.
- At 50M delta events/sec, Kafka would need ~50 brokers just for ephemeral data that has zero replay value.
- NATS core pub/sub is fire-and-forget with ~0.1ms latency. If a subscriber misses a delta, it doesn't matter — the `completed` event has the full content.

**Tier 2: Durable events — Kafka as primary store, PG as cold storage**

All non-delta events go to Kafka. These are the events that matter for replay, history, and API queries:

```
                    Write path (hot)                    Read path
                    ─────────────                       ─────────
Worker completes    Kafka topic: events                 SSE stream subscribes
  turn/tool/msg ──► partitioned by session_id ────────► to Kafka partition
  │                      │                                   │
  │                      │ consumer group                    │ (real-time)
  │                      ▼                                   │
  │                 Batch writer                              │
  │                   (async)                                 │
  │                      │                                   │
  │                      ▼                                   │
  │                 PostgreSQL                                │
  │                 events table ◄───────────────────────── REST API
  │                 (cold storage)            (historical queries,
  │                                           dashboard, search)
  │
  └─► gRPC: update session status, workflow state → PostgreSQL (directly)
```

**What changes in the codebase:**

1. **`EventService.emit()`** — split: ephemeral events go to in-memory/NATS broadcast (no storage), durable events go to Kafka topic. Current code path (`db.create_event()` → PG INSERT) is removed from the hot path.

2. **`EventNotificationBroadcaster`** — replaced by two channels:
   - Ephemeral: NATS core pub/sub (or in-process `broadcast::channel` for single-instance)
   - Durable: Kafka consumer that pushes to SSE streams

3. **SSE event streams** — subscribe to both channels. Ephemeral events arrive with ~0.1ms latency (NATS). Durable events arrive with ~2-5ms latency (Kafka). Client sees a unified ordered stream.

4. **REST `GET /v1/sessions/{id}/events`** — reads from PG only. Delta events are excluded (they were never stored). This is already supported: the API has `exclude_types` filtering, and clients can opt out of deltas with `?exclude=output.message.delta`.

5. **`events` table in PG** — much smaller without deltas. The heaviest index (`idx_events_session_sequence`) sees ~10x less write pressure since deltas were ~90% of event volume.

**Volume reduction:**

| Event category | Events/turn (typical) | % of total | Storage tier |
|---|---|---|---|
| Deltas (message, thinking, tool output) | ~50 | ~80% | Ephemeral (none) |
| Lifecycle (turn.started, turn.completed) | 2 | ~3% | Durable (Kafka → PG) |
| Messages (input, output.completed) | 2 | ~3% | Durable (Kafka → PG) |
| Tool calls (started, completed) | ~5 | ~8% | Durable (Kafka → PG) |
| Other (status, metadata) | ~3 | ~5% | Durable (Kafka → PG) |

**At 1M agents, this changes the Kafka sizing dramatically:**

| Metric | All events (original) | Durable only (revised) |
|---|---|---|
| Events/sec peak | 5M/sec | ~500K/sec |
| Kafka partitions | 256 | 64 |
| Kafka brokers | 3-5 | 3 |
| Kafka retention (7d) | ~3TB | ~300GB |
| PG backfill rate | 100K/sec | ~50K/sec |

**Sequence numbering:**

Durable events use Kafka partition offset as sequence (naturally ordered per session). Ephemeral events use a lightweight in-memory counter per session in the worker — no PG advisory lock, no coordination. If a worker restarts mid-stream, the sequence resets, but that's fine — deltas are consumed in real-time and never queried by sequence.

**Retention tiers (durable events only):**

| Tier | Storage | Retention | Purpose |
|---|---|---|---|
| Hot | Kafka | 7 days | Real-time SSE, recent session replay |
| Warm | PostgreSQL | 90 days | REST API, dashboard queries, search |
| Cold | S3/object store | Indefinite | Audit trail, compliance, replay |

The Kafka → PG consumer batches durable events (COPY protocol). The Kafka → S3 consumer runs daily, compacting events into Parquet files by org/session for cheap long-term storage. Ephemeral events are never written to any persistent store.

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

1. **PostgreSQL for durable state only** — PostgreSQL is the right tool for sessions, agents, orgs, and workflow state (CRUD with ACID). It is the wrong tool for high-throughput task queuing, event streaming, and pub/sub notifications. At 1M scale, stop forcing PG to do jobs better served by purpose-built systems.

2. **Right tool for each job:**

   | Job | Current (PG) | At 1M scale | Why |
   |-----|---|---|---|
   | **Durable state** (sessions, agents, orgs, workflow instances) | PostgreSQL | PostgreSQL (sharded by org) | ACID, relational queries, proven |
   | **Task queue** | `durable_task_queue` with `SKIP LOCKED` | **NATS JetStream** | Purpose-built work queue. No row locking, no index bloat, built-in consumer groups, backpressure, replay. PG `SKIP LOCKED` hits hot-page contention at 100K+ pending tasks |
   | **Event log** (session events, tool calls) | `events` table, INSERT-per-event | **Kafka/Redpanda** → async consumer writes to PG | Append-optimized, partitioned by session, millions/sec. PG WAL bottlenecks at ~100K inserts/sec. Cold events still land in PG for queries |
   | **Push notifications** (task available, event available) | `NOTIFY/LISTEN` | **NATS pub/sub** | Fan-out to 1000+ subscribers. PG NOTIFY is single-threaded sender, single channel, no partitioning |
   | **Workflow event sourcing** | `durable_workflow_events` | PostgreSQL with aggressive snapshotting | Keep in PG — low volume per workflow (100s of events). Snapshot every 100 events, archive old ones. Not the bottleneck |

3. **Workers remain stateless** — No architectural change to worker model. Just more of them with partition affinity.

4. **Org-based sharding key** — Natural tenant boundary. No cross-shard queries needed for normal operations. Cross-org queries (admin dashboards) go through aggregation layer.

5. **Incremental adoption** — Each tier is independently deployable. A single-org deployment stays on pure PostgreSQL forever (Tier 1). NATS and Kafka are only added when crossing 50K concurrent agents.

6. **PostgreSQL as cold storage, not hot path** — At 1M, PG serves reads (session history, agent config, dashboards) and receives async bulk writes (batched events from Kafka consumer). It never sits in the hot path of task dispatch or real-time event delivery.

## Data Flow at 1M (Revised)

```
                         ┌──────────────────────────┐
                         │      LLM Providers        │
                         └────────────▲─────────────┘
                                      │
┌─────────────────────────────────────┼─────────────────────────────────────┐
│                              Workers (1000+)                               │
│                                     │                                      │
│  ┌──────────────────────────────────┼──────────────────────────────────┐  │
│  │  1. Claim task from NATS JetStream (consumer group, partitioned)   │  │
│  │  2. Execute: Input → Reason (LLM) → Act (tools)                    │  │
│  │  3. Publish events to Kafka topic (session events)                 │  │
│  │  4. ACK task in NATS                                                │  │
│  │  5. Publish task notifications to NATS (if spawned child tasks)    │  │
│  └──────────────────────────────────┼──────────────────────────────────┘  │
│                                     │                                      │
└─────────────────────────────────────┼─────────────────────────────────────┘
                                      │
          ┌───────────────────────────┼───────────────────────────┐
          │                           │                           │
   ┌──────▼──────┐           ┌────────▼────────┐         ┌───────▼───────┐
   │ NATS         │           │ Kafka/Redpanda  │         │ PostgreSQL    │
   │ JetStream    │           │                 │         │ (sharded)     │
   │              │           │ • events topic  │         │               │
   │ • task queue │           │   (by session)  │         │ • sessions    │
   │   (by type   │           │ • async PG      │         │ • agents      │
   │    + org)    │           │   writer        │─batch──▶│ • events      │
   │ • pub/sub    │──push────▶│ • SSE fan-out   │         │   (cold)      │
   │   (notifs)   │           │                 │         │ • workflows   │
   └──────────────┘           └─────────────────┘         │ • orgs        │
                                      │                   └───────────────┘
                                      │ push
                               ┌──────▼──────┐
                               │ Control     │
                               │ Plane (×4)  │
                               │             │
                               │ • REST API  │
                               │ • SSE out   │
                               │ • gRPC      │
                               └─────────────┘
```

**Hot path** (task dispatch + event delivery): NATS + Kafka only. No PostgreSQL in the loop.

**Warm path** (session state reads during execution): PostgreSQL via gRPC, with connection pooling. Workers read session config, write workflow state updates.

**Cold path** (dashboards, history, search): PostgreSQL read replicas. Kafka consumer backfills events asynchronously.

## Risks

| Risk | Mitigation |
|------|-----------|
| Cross-shard operations (admin queries, global search) | Aggregation API that fans out to all shards |
| Shard hotspots (one org with 500K agents) | Sub-org sharding key (org_id + session_id hash) |
| NATS as new SPOF | 3-node cluster, fallback to PostgreSQL polling |
| Kafka consumer lag (events delayed to PG) | Monitor consumer lag, alert at >10s. Acceptable for cold storage — SSE reads from Kafka directly |
| Operational complexity (3 stateful systems) | NATS + Kafka only deployed at Tier 2/3. Single-org stays on PG-only |
| Migration complexity (partitioning existing tables) | pg_partman for online partition creation, backfill in background |
| LLM cost at 1M scale | Tiered models (fast/cheap for simple tasks, expensive for complex), aggressive caching of common prompts |
