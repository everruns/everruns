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

**Tier 1: Ephemeral events — pub/sub, never stored**

Delta events (`output.message.delta`, `reason.thinking.delta`, `tool.output.delta`) go through pub/sub — never persisted to any store. The pub/sub backend scales with deployment size:

| Scale | Ephemeral pub/sub | Why |
|---|---|---|
| Single instance (dev) | In-process `broadcast::channel` | Zero deps, ~0ms |
| Multi-instance, ≤50K agents | **Valkey pub/sub** (already deployed, `fred` crate) | Already in stack for rate limiting. Single Valkey handles ~200K msg/sec realistic delta load. Channel per session: `session:{id}:deltas` |
| Multi-instance, 500K+ agents | **NATS core pub/sub** | Valkey sharded pub/sub maxes out. NATS handles 10M+ msg/sec natively across 3-node cluster. Subject per session: `deltas.session.{id}` |

Valkey is the pragmatic default — it's already deployed (`VALKEY_URL`), the `fred` client is already a dependency, and adding `PUBLISH`/`SUBSCRIBE` for deltas is ~50 lines of code. NATS is only added when delta volume exceeds what Valkey can fan out.

**Delta delivery path: workers stay gRPC-only**

Key architecture question: should workers publish directly to NATS/Valkey for deltas?

**No.** Workers must not know about NATS, Valkey, or Kafka. The current principle — workers communicate exclusively via gRPC to the control plane — is load-bearing for deployment simplicity and security (workers don't need pub/sub credentials, don't need network access to message brokers, and can run in isolated environments). Breaking this creates a distributed system inside the worker.

Instead, the control plane is the **routing point** that decides where events go:

```
Current (every delta is a blocking gRPC round-trip → PG INSERT):

  Worker ReasonAtom                Control Plane              PostgreSQL
  ─────────────────                ─────────────              ──────────
  LLM token chunk
    → event_emitter.emit()
      → gRPC EmitEvent ──────────► EventService.emit()
                                     → db.create_event() ──► INSERT INTO events
                                     ← EventRow            ◄─ (wait for commit)
      ← Event response ◄──────────
    (blocks until PG commit)
    next token...


Proposed (deltas fire-and-forget via gRPC, control plane routes to pub/sub):

  Worker ReasonAtom                Control Plane              Pub/Sub
  ─────────────────                ─────────────              ───────
  LLM token chunk
    → event_emitter.emit()
      → gRPC EmitEvent ──────────► EventService.emit()
        (fire-and-forget stream)     is_ephemeral()? → yes
                                     → NATS/Valkey PUBLISH ──► SSE subscribers
                                     ← ack (no PG, no wait)
      ← lightweight ack ◄──────────
    (non-blocking, ~1ms)                                    (not stored)
    next token...

  LLM [done]
    → event_emitter.emit(completed)
      → gRPC EmitEvent ──────────► EventService.emit()
                                     is_ephemeral()? → no
                                     → Kafka produce ──► PG consumer (async)
                                     ← Event
      ← Event response ◄──────────
```

The change is **entirely inside the control plane's `EventService.emit()`** — workers call the same `gRPC EmitEvent` they always did. The control plane routes based on event type:

- Ephemeral → pub/sub PUBLISH, return lightweight ack (no PG)
- Durable → Kafka produce, return stored event

**Optimization: gRPC streaming for deltas**

The current `EmitEvent` RPC is unary (request-response per event). At 100ms batch interval, that's 10 gRPC round-trips/sec per active agent — fine at small scale, but 10M round-trips/sec at 1M agents is excessive.

Better: use a **bidirectional gRPC stream** for delta events. The worker opens one stream per session and pushes deltas without waiting for responses. The control plane reads from the stream and publishes to pub/sub in bulk.

```protobuf
// New: streaming RPC for ephemeral events
service WorkerService {
    // Existing: unary RPC for durable events
    rpc EmitEvent(EmitEventRequest) returns (EmitEventResponse);

    // New: streaming RPC for ephemeral delta events
    // Worker pushes deltas, control plane acks periodically (not per-message)
    rpc StreamEphemeralEvents(stream EmitEventRequest) returns (stream EphemeralAck);
}

message EphemeralAck {
    // Periodic ack — not per-event. Just confirms the stream is alive.
    uint64 events_received = 1;
}
```

The `ReasonAtom` in the worker detects ephemeral events and routes to the stream:

```rust
// In core: EventEmitter trait stays the same
// In worker: GrpcEventEmitter splits internally

impl EventEmitter for GrpcEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        if request.is_ephemeral() {
            // Push to open gRPC stream (fire-and-forget, non-blocking)
            self.ephemeral_stream.send(request).await?;
            // Return synthetic Event (no PG id, no sequence)
            Ok(request.to_ephemeral_event())
        } else {
            // Existing unary gRPC call for durable events
            self.emit_durable(request).await
        }
    }
}
```

**Workers don't know about NATS/Valkey/Kafka.** They know about gRPC. The control plane decides the routing.

```
Worker                     Control Plane                 Infrastructure
──────                     ─────────────                 ──────────────
                                                          ┌─ NATS/Valkey (deltas)
gRPC EmitEvent ──────────► EventService.emit() ──────────►├─ Kafka (durable events)
gRPC StreamEphemeral ────► EphemeralRouter.publish() ────►└─ PG (cold storage)
                                                         (workers don't know about any of this)
```

```rust
/// EventService.emit() — split by event durability
pub async fn emit(&self, request: EventRequest) -> Result<Event> {
    Self::validate_event_type_consistency(&request)?;

    if request.is_ephemeral() {
        // Ephemeral: broadcast only, no storage
        // Sequence assigned from in-memory counter (per session)
        let event = self.to_ephemeral_event(request);
        // Valkey PUBLISH (multi-instance) or broadcast::channel (single instance)
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
- Valkey pub/sub is fire-and-forget with ~0.2ms latency, and it's already deployed. If a subscriber misses a delta, it doesn't matter — the `completed` event has the full content.
- At extreme scale (500K+ agents), NATS replaces Valkey for deltas — same fire-and-forget semantics but 10x higher throughput ceiling.

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

3. **SSE event streams** — subscribe to both channels. Ephemeral events arrive with ~0.2ms latency (Valkey pub/sub). Durable events arrive with ~2-5ms latency (Kafka). Client sees a unified ordered stream.

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

## Implementation Phases (Revised)

The original phases (Tier 1/2/3) map to infrastructure layers. These revised phases are ordered by **impact per effort** and build on each other without throwaway code.

### SSE Architecture: NATS for real-time, PG for replay

The current SSE stream has a fundamental problem: it **polls PostgreSQL in a loop** (`event_service.list(session_id, since_id)` every 100-500ms). PG NOTIFY is just a waker ("poll now"), not a delivery mechanism. This means:

1. Every SSE connection holds a tokio task that queries PG repeatedly
2. At 10K SSE connections × 2-10 polls/sec = 20-100K queries/sec just for SSE
3. All events (including deltas) must be persisted before SSE can see them

**Replace with: NATS for all real-time SSE delivery, PG for reconnection replay only.**

NATS JetStream (not core pub/sub) is the right fit because it gives **both** real-time push delivery **and** replay from a position — which solves the SSE `since_id` reconnection problem that pure fire-and-forget pub/sub can't handle.

```
                    Normal operation (connected)     Reconnection (since_id)
                    ────────────────────────────     ───────────────────────

EventService.emit()                                  Client reconnects with since_id
  │                                                    │
  ├─ All events ──► NATS JetStream                     ├─ Durable events: PG query
  │                 stream: events.{session_id}        │   SELECT * FROM events
  │                 │                                  │   WHERE session_id = $1
  │                 │ (real-time push)                 │   AND sequence > $2
  │                 ▼                                  │
  │                 SSE client                          ├─ Ephemeral events: missed
  │                                                    │   (acceptable — completed
  │                                                    │    event has full text)
  ├─ Durable ────► PG INSERT (for replay/REST)         │
  │                                                    └─► Resume NATS subscription
  └─ Ephemeral ──► NATS only (no PG)
```

**How the SSE stream changes:**

```
Current (poll PG):
  loop {
    events = event_service.list(session_id, since_id)  // PG query
    if events.empty() {
      select! {
        pg_notify.notified() => {}  // wake up early
        sleep(backoff) => {}        // fallback
      }
    }
    send_sse(events)
  }

Proposed (subscribe NATS):
  // On connect: replay missed durable events from PG (if since_id provided)
  if since_id.is_some() {
    missed = event_service.list(session_id, since_id)  // PG query, ONE TIME
    send_sse(missed)
  }

  // Then: subscribe to NATS stream for real-time push
  let consumer = nats.subscribe("events.{session_id}").await;
  loop {
    select! {
      event = consumer.next() => send_sse(event)  // push, no polling
      _ = cycle_timer => send_disconnecting()
    }
  }
```

**Key differences:**

| Aspect | Current (PG poll) | Proposed (NATS + PG) |
|---|---|---|
| Delivery | Poll PG every 100-500ms | NATS push, ~1ms |
| PG load per SSE connection | 2-10 queries/sec continuous | 0-1 query on connect (replay) |
| Delta delivery | PG INSERT + PG query | NATS only, no PG |
| Reconnection (`since_id`) | PG query (same as normal) | PG query (one-time catch-up), then NATS |
| Cross-instance | PG NOTIFY waker (works) | NATS subscription (native) |
| DB connections consumed | 1 per SSE stream (pool contention) | 0 during normal streaming |

**Why NATS JetStream, not NATS core pub/sub:**

NATS core pub/sub is fire-and-forget — if the SSE connection drops for 2 seconds, events during that window are lost. NATS JetStream retains messages and supports consumer replay from a sequence position. This maps directly to SSE's `since_id` reconnection:

- NATS JetStream stream: `events.{session_id}`, retention per session (e.g., 1 hour or 10K messages)
- On SSE connect: create ephemeral JetStream consumer, deliver from "last received" position
- On SSE reconnect: consumer replays from where it left off (NATS handles this)
- Ephemeral events (deltas) are in JetStream briefly (~minutes) then expire. Long-term replay (hours/days) falls back to PG for durable events only

**What about Valkey?** Valkey stays for rate limiting (its current job). Delta pub/sub moves to NATS JetStream since we need JetStream anyway for replay semantics. No point running two pub/sub systems.

**Dev mode fallback:** When `NATS_URL` is not set (dev mode, `just start-dev`), all events go to PG as today. The SSE stream falls back to the current poll loop. Zero regression.

### Event Delivery Abstraction

The event delivery layer follows the same pattern as `StorageBackend` (enum dispatch, `Postgres`/`InMemory` variants). This answers three deployment questions:

#### 1. Dev mode: single binary, no NATS required

Dev mode (`just start-dev`) runs in-memory with no external services. The event delivery backend has an `InMemory` variant that uses in-process `broadcast::channel` — same as the current `EventNotificationBroadcaster` but carrying actual event data instead of just wake signals.

```rust
/// Event delivery backend — same dispatch pattern as StorageBackend
pub enum EventDelivery {
    /// Production: NATS JetStream for real-time push + replay
    Nats(NatsEventDelivery),
    /// Dev mode: in-process broadcast channels, zero external deps
    InMemory(InMemoryEventDelivery),
}

impl EventDelivery {
    pub async fn from_env() -> Self {
        match std::env::var("NATS_URL") {
            Ok(url) => Self::Nats(NatsEventDelivery::connect(&url).await),
            Err(_) => {
                tracing::info!(
                    "NATS_URL not set — using in-memory event delivery. \
                     Set NATS_URL for multi-instance SSE and ephemeral delta support."
                );
                Self::InMemory(InMemoryEventDelivery::new())
            }
        }
    }

    pub async fn publish(&self, session_id: Uuid, event: &Event) { ... }
    pub async fn subscribe(&self, session_id: Uuid) -> EventSubscription { ... }
}

/// In-memory: partitioned broadcast channels (same process only)
pub struct InMemoryEventDelivery {
    channels: [broadcast::Sender<Event>; 16],  // partitioned by session_id
}

/// NATS: JetStream streams with per-session subjects
pub struct NatsEventDelivery {
    client: async_nats::Client,
    jetstream: async_nats::jetstream::Context,
}
```

**Result**: `just start-dev` works exactly as today. Single binary, no NATS, no Docker. The in-memory variant carries event data through broadcast channels — SSE subscribes to the channel instead of polling PG. Same real-time behavior, just in-process.

#### 2. No-Docker environments: NATS as a single binary

NATS server is a single static binary (~20MB), same deployment model as PostgreSQL and Valkey. Add to `scripts/lib/infra.sh` alongside existing `start_postgres` and `start_valkey`:

```bash
# scripts/lib/infra.sh

NATS_PORT="${NATS_PORT:-4222}"
NATS_DATA_DIR="${LOCAL_DATA_DIR}/nats-${NATS_PORT}"
NATS_PIDFILE="${NATS_DATA_DIR}/nats.pid"
NATS_LOGFILE="${NATS_DATA_DIR}/nats.log"

detect_nats_bin() {
  if command -v nats-server &> /dev/null; then
    NATS_SERVER="nats-server"
    return 0
  fi
  echo "⚠️  nats-server not found. NATS will be skipped (SSE falls back to PG polling)."
  return 1
}

start_nats() {
  if ! detect_nats_bin; then return 1; fi
  if nats_is_running; then
    echo "   ✅ NATS already running (port ${NATS_PORT})"
    return 0
  fi

  echo "   Starting NATS with JetStream on port ${NATS_PORT}..."
  mkdir -p "$NATS_DATA_DIR"
  $NATS_SERVER \
    --port "$NATS_PORT" \
    --jetstream \
    --store_dir "$NATS_DATA_DIR/jetstream" \
    --pid "$NATS_PIDFILE" \
    --log "$NATS_LOGFILE" \
    --daemon &

  # Wait for ready
  for i in {1..10}; do
    if nats-server --help >/dev/null 2>&1 && \
       curl -s "http://localhost:${NATS_PORT}/healthz" | grep -q ok; then
      echo "   ✅ NATS started on localhost:${NATS_PORT}"
      export NATS_URL="nats://localhost:${NATS_PORT}"
      return 0
    fi
    sleep 1
  done
}
```

`just start-all` starts PG + Valkey + NATS. `just start-dev` skips all three (in-memory). Same pattern, same developer experience. NATS installation is one line: `brew install nats-server` or `apt install nats-server` or download from https://nats.io/download/.

#### 3. Cloud-native pub/sub abstraction (future, not required)

The `EventDelivery` enum can be extended with cloud-native variants without changing any calling code:

```rust
pub enum EventDelivery {
    Nats(NatsEventDelivery),
    InMemory(InMemoryEventDelivery),
    // Future cloud-native variants:
    // GcpPubSub(GcpPubSubEventDelivery),    // Google Cloud Pub/Sub
    // AwsSns(AwsSnsEventDelivery),            // AWS SNS/SQS
    // AzureServiceBus(AzureEventDelivery),    // Azure Service Bus
}
```

The trait surface is minimal — `publish(session_id, event)` and `subscribe(session_id) -> Stream<Event>`. Any pub/sub system that supports:
- Per-topic/subject publishing
- Per-subscription message delivery
- Short-term message retention (for reconnection replay)

...can implement this. GCP Pub/Sub, AWS SNS+SQS, Azure Service Bus all qualify. The only NATS-specific feature we rely on is JetStream's replay-from-position for SSE reconnection. Cloud equivalents: GCP Pub/Sub `seek()`, SQS with message deduplication, etc.

Not needed now — NATS is the right default for self-hosted and cloud deployments. But the abstraction doesn't lock us in.

### Phase 1: NATS JetStream for SSE delivery — ~2-3 weeks

This is a bigger first phase but eliminates more throwaway code. NATS becomes the single real-time delivery layer for all events.

**Steps:**

1. Deploy NATS with JetStream enabled (single node for dev, 3-node for prod)
2. Add `async-nats` crate dependency to `crates/server`
3. Add `is_ephemeral()` to `EventRequest` — classify delta event types
4. `EventService.emit()`:
   - All events → publish to NATS JetStream stream `events.{session_id}`
   - Durable events → also `db.create_event()` (PG INSERT, unchanged)
   - Ephemeral events → NATS only, skip PG
5. Rewrite SSE stream (`crates/server/src/api/events.rs`):
   - On connect with `since_id`: one-time PG query for missed durable events, then subscribe to NATS
   - On connect without `since_id`: subscribe to NATS directly
   - Replace poll loop with NATS consumer `next()` — no backoff, no PG polling
6. Remove `EventNotificationBroadcaster` (PG NOTIFY listener) — no longer needed
7. Dev mode: if `NATS_URL` not set, fall back to current PG poll (zero regression)
8. Load test: validate delta throughput, SSE latency, reconnection replay

**Impact**: Eliminates ~80% of PG event writes (deltas). Eliminates PG polling from SSE entirely. Horizontally scalable. Single delivery mechanism for all event types.

### Phase 2: Fire-and-forget gRPC for ephemeral events — ~1 day ✅

Worker's `GrpcEventEmitter` now routes by event type:
- **Ephemeral events** (deltas, LLM generation): return synthetic Event immediately,
  fire gRPC `EmitEvent` in a background tokio task. No mutex lock held during streaming.
- **Durable events**: blocking gRPC round-trip (unchanged, needs DB-assigned id + sequence).

No proto changes needed. The existing `EmitEvent` unary RPC works for both paths — the
difference is whether the worker waits for the response. Server-side `EventService.emit()`
already routes ephemeral events to EventDelivery only (Phase 1).

**Impact**: `ReasonAtom`'s delta emission loop no longer blocks on gRPC round-trips.
Workers go from ~10 blocking calls/sec (100ms batch interval × mutex lock × network RTT)
to fire-and-forget with ~0ms overhead per delta.

See `crates/worker/src/grpc_adapters.rs` — `GrpcAdapter::emit_ephemeral()`.

### Phase 3: NATS pub/sub for task notifications — ~1 day ✅

NATS is already deployed from Phase 1. Replace PG `NOTIFY/LISTEN` for task availability
notifications with NATS pub/sub when `NATS_URL` is set.

**What changed:**
- New `TaskBroadcaster` enum wrapping PG and NATS backends with the same API
- `NatsTaskNotificationBroadcaster`: subscribes to `task.available.>`, forwards to
  broadcast channel. Workers are transparent — they still get gRPC push notifications.
- `TaskBroadcaster::from_env()` selects NATS if `NATS_URL` is set, falls back to PG NOTIFY
- No worker code changes. No task claim changes. PG `SKIP LOCKED` remains the claim path.
- `notify_task_available()` publishes to NATS subjects (PG backend uses DB triggers instead)

**What didn't change (deferred):**
- Task enqueue still goes to PG only (NATS notification is supplementary)
- Workers still claim via gRPC → PG `SELECT FOR UPDATE SKIP LOCKED`
- Moving task claiming itself to NATS consumer groups is future work if PG contention
  becomes a bottleneck at >100K concurrent tasks

See `crates/server/src/nats_task_notifications.rs` and `crates/server/src/task_notifications.rs`.

**Impact**: Task notification delivery off PG NOTIFY. Lower latency (~1ms vs ~30ms),
multi-instance support, no PG connection dedicated to LISTEN. Gated on `NATS_URL`.

### Phase 3.5: NATS end-to-end operational readiness

Phases 1-3 built the library-level NATS integration. Phase 3.5 makes it fully operational:
infrastructure scripts, all notification paths wired, documentation, integration tests.

#### A. Infrastructure: `start-all` starts NATS, exports `NATS_URL`

**scripts/lib/services.sh** — `start-all` must:
- Call `start_nats` from `infra.sh` (already implemented)
- Export `NATS_URL=$NATS_URL_DEFAULT` alongside `VALKEY_URL` and `DATABASE_URL`
- Server process receives `NATS_URL` → `EventDelivery::Nats` + `TaskBroadcaster::Nats`

`start-dev` stays unchanged — no NATS, no `NATS_URL`, pure PG behavior.

#### B. Wire task notifications in ALL enqueue paths

**Currently wired (gRPC only):**
- `grpc_service/worker_service_impl.rs` — `enqueue_durable_task()` ✓
- `grpc_service/worker_service_impl.rs` — `fail_durable_task()` (retry) ✓

**Missing — must add `notify_task_available()` calls:**
- `crates/server/src/api/durable.rs` — REST `POST /v1/durable/tasks` standalone enqueue.
  Add `TaskBroadcaster` to durable `AppState`, call after `store.enqueue_task()`.
- `crates/durable/src/scheduler/mod.rs` — scheduled task triggers.
  Scheduler enqueues tasks via `store.enqueue_task()`. PG trigger handles PG NOTIFY,
  but NATS needs an explicit publish. Pass `TaskBroadcaster` to scheduler or publish
  from the store wrapper.
- `crates/durable/src/worker/pool.rs` — `reclaim_stale_tasks()` moves tasks back to pending.
  PG trigger handles PG NOTIFY. For NATS, publish after reclaim returns reclaimed count > 0.

#### C. Docker Compose: add NATS service

**examples/docker-compose-full.yaml** — add NATS JetStream container:
```yaml
nats:
  image: nats:2-alpine
  command: ["--jetstream", "--store_dir", "/data"]
  ports:
    - "${NATS_PORT:-4222}:4222"
  volumes:
    - nats-data:/data
  healthcheck:
    test: ["CMD", "nats-server", "--help"]
    interval: 5s
    timeout: 3s
    retries: 3
```
Server service env: `NATS_URL: nats://nats:4222`

#### D. Environment variable documentation

**docs/sre/environment-variables.md** — add NATS section:
- `NATS_URL` — NATS connection URL (e.g., `nats://localhost:4222`). When set, enables
  NATS JetStream for event delivery and task notifications. When unset, system uses
  PostgreSQL NOTIFY and in-memory broadcast (zero behavioral change).
- `NATS_PORT` — Port for local NATS server (default: 4222, or `PORT_PREFIX22` with prefix).

#### E. Integration test: SSE push delivery with NATS

Add a test in `crates/server/tests/` that:
1. Starts with `NATS_URL` pointing to a test NATS server (or embedded)
2. Creates a session, sends a message
3. Subscribes to SSE stream
4. Verifies `output.message.completed` arrives via push (not PG poll)
5. Verifies ephemeral events (`output.message.delta`) are NOT in PG events table
6. Verifies durable events ARE in PG events table

If running a real `nats-server` in CI is complex, test with `EventDelivery::InMemory`
as a proxy — the SSE `Streaming` phase uses the same `EventSubscription::recv()` API.

#### F. Durable dashboard: event delivery indicator

Add to the durable overview page (`apps/ui/src/app/(main)/durable/`):
- Badge showing event delivery backend: "NATS JetStream" or "In-Memory (PG fallback)"
- Expose via existing `/v1/durable/health` endpoint: add `event_delivery: "nats" | "in_memory"`
  field to `SystemHealth` response.

#### G. Update specs and AGENTS.md

- `specs/architecture.md` — already has EventDelivery section ✓
- `docs/proposals/parallel-execution-1m.md` — this section ✓
- `AGENTS.md` — update Local Dev section: mention `start-all` starts NATS,
  `NATS_URL` controls event delivery backend
- `specs/events.md` — note ephemeral event classification
  and that deltas skip PG when NATS is active

#### Implementation order

1. **services.sh + infra.sh** — export `NATS_URL` in `start-all` (~30 min)
2. **Wire missing notification paths** — REST durable API, scheduler, reclaim (~2-3 hours)
3. **Docker compose** — add NATS service (~30 min)
4. **Env var docs** — document `NATS_URL`, `NATS_PORT` (~30 min)
5. **Integration test** — SSE push delivery test (~2-3 hours)
6. **Dashboard indicator** — health endpoint + UI badge (~1-2 hours)
7. **Spec updates** — AGENTS.md, events specs (~30 min)

### Phase 4: Kafka for durable event cold storage (future consideration)

At extreme scale (500K+ agents), synchronous PG INSERT for every durable event may bottleneck. Kafka/Redpanda would decouple the write path: durable events publish to Kafka, an async consumer backfills PG via batched COPY. SSE continues reading from NATS (unchanged). PG becomes cold storage for REST API and dashboards with 2-5s acceptable lag. Evaluate after Phase 3 based on measured PG write throughput.

### Phase 5: Horizontal sharding (future consideration)

For 1M+ concurrent agents, shard the control plane and PostgreSQL by org_id. Multiple CP instances each own a partition range, routed by load balancer. Includes PG hash partitioning (32 partitions), worker partition affinity, shard-local NATS subjects, and an LLM request router with key pooling. Evaluate after Phase 3 based on measured single-instance ceiling.

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

## Data Flow (After Phase 3)

```
                         ┌──────────────────────────┐
                         │      LLM Providers        │
                         └────────────▲─────────────┘
                                      │
┌─────────────────────────────────────┼─────────────────────────────────────┐
│                              Workers                                       │
│                                     │                                      │
│  ┌──────────────────────────────────┼──────────────────────────────────┐  │
│  │  1. Claim task from NATS JetStream (consumer group)                │  │
│  │  2. Execute: Input → Reason (LLM) → Act (tools)                    │  │
│  │  3. gRPC EmitEvent / StreamEphemeral → Control Plane               │  │
│  │  4. ACK task in NATS                                                │  │
│  └──────────────────────────────────┼──────────────────────────────────┘  │
│                                     │ gRPC only                            │
└─────────────────────────────────────┼─────────────────────────────────────┘
                                      │
                               ┌──────▼──────┐
                               │ Control     │
                               │ Plane       │
                               │             │
                               │ • REST API  │
                               │ • SSE out   │
                               │ • gRPC in   │
                               └──┬───────┬──┘
                                  │       │
                    ┌─────────────▼─┐   ┌─▼─────────────┐
                    │ NATS JetStream │   │ PostgreSQL     │
                    │                │   │                │
                    │ • SSE delivery │   │ • sessions     │
                    │   (all events) │   │ • agents       │
                    │ • task queue   │   │ • durable      │
                    │ • notifications│   │   events (cold)│
                    └────────────────┘   │ • workflows    │
                                         │ • orgs         │
                                         └────────────────┘
```

**Hot path** (task dispatch + event delivery): NATS only. No PostgreSQL in the loop.

**Warm path** (session state, workflow state): PostgreSQL via gRPC. Durable event writes still go to PG synchronously (until Phase 4).

**Cold path** (dashboards, history, REST API): PostgreSQL.

## Risks

| Risk | Mitigation |
|------|-----------|
| NATS as new SPOF | 3-node cluster, fallback to PostgreSQL polling when `NATS_URL` not set |
| NATS JetStream storage growth | Per-session retention limits (1 hour or 10K messages), ephemeral events expire quickly |
| SSE reconnection gap | Durable events replayed from PG on reconnect. Missed ephemeral deltas acceptable (completed event has full text) |
| Operational complexity (PG + NATS) | NATS only deployed when needed. Dev mode stays PG-only |
| LLM cost at scale | Tiered models (fast/cheap for simple tasks, expensive for complex), aggressive caching of common prompts |
