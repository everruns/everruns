# Redis Requirements Analysis

## Abstract

Analysis of whether everruns needs Redis (or alternatives like Valkey, DragonflyDB, KeyDB) given the multitenant architecture. **Verdict: not yet, but approaching the threshold for two specific use cases.**

## Current State: PostgreSQL Does Everything

Today, PostgreSQL is the sole stateful dependency. It handles:

| Concern | Mechanism |
|---------|-----------|
| Persistence | Tables (agents, sessions, events, etc.) |
| Task queue | `durable_task_queue` + `SKIP LOCKED` |
| Push notifications | `NOTIFY/LISTEN` (`event_available`, `task_available`) |
| Scheduled jobs | `durable_schedules` + polling + `SKIP LOCKED` |
| Liveness detection | Heartbeat rows + stale task reclamation |
| Multitenancy isolation | `WHERE org_id = $org_id` on every query |

In-process only (not shared across instances):

| Concern | Mechanism |
|---------|-----------|
| Rate limiting | `governor` crate with `DashMap` (per-IP, auth endpoints only) |
| SSE fan-out | `tokio::broadcast` channels (fed by PgListener) |
| Metrics | Ring buffer (360 points, 10s resolution, 1 hour window) |

## Where Redis Would Help (Ranked by Urgency)

### 1. Distributed Rate Limiting — Medium Priority

**Problem:** Rate limiting is per-instance. With `EXPECTED_INSTANCES=N`, each instance independently enforces limits. A client hitting different instances behind a load balancer gets N× the intended budget.

**Current scope:** Auth endpoints only (login 10/min, register 5/min, refresh 30/min). Low risk at current scale.

**When it matters:**
- Per-org API rate limits (tenant abuse prevention)
- Per-org LLM token budgets (TPM/RPM across workers)
- Per-org SSE connection limits (currently divided by N, but inexact)

**Redis solution:** Sliding-window or token-bucket via `MULTI`/`EXEC` or Lua scripts. Sub-millisecond overhead. Every major rate-limiting crate (governor, tower-governor) supports Redis backends.

**Alternative without Redis:** PostgreSQL advisory locks or a `rate_limit_counters` table with `ON CONFLICT DO UPDATE`. Works but adds write pressure to the primary DB for high-frequency operations.

### 2. Cross-Instance Cache — Low Priority (For Now)

**Problem:** No shared cache layer. Every instance hits PostgreSQL for repeated reads (agent config, LLM provider config, capability lookups).

**Current mitigation:** Connection pooling + PostgreSQL shared buffers handle this fine at moderate scale. Agent/provider configs change rarely and are small.

**When it matters:**
- Hundreds of orgs with active sessions, each fetching agent configs per turn
- `GetTurnContext` gRPC calls become a hot path (every tool call fetches context)
- Read replicas are one option, but a cache is simpler for immutable-ish data

**Redis solution:** Read-through cache with short TTL (30-60s). Cache key: `org:{org_id}:agent:{agent_id}`. Invalidation via PostgreSQL trigger → NOTIFY → cache evict.

**Alternative without Redis:** In-process LRU cache (e.g., `moka` crate) with TTL. Works well for single-instance. For multi-instance, stale reads up to TTL are acceptable for config data. **This is probably sufficient for now.**

### 3. Pub/Sub Beyond PostgreSQL NOTIFY — Low Priority

**Problem:** PostgreSQL `NOTIFY` has limitations:
- 8000-byte payload limit (workaround: send IDs, fetch full data)
- No persistence (missed if listener disconnects between reconnects)
- No topic filtering (all instances receive all notifications)

**Current mitigation:** Connection cycling (5-min SSE, 10-min durable) + `since_id` resume makes missed NOTIFYs a minor latency blip, not data loss. The system already handles this gracefully.

**When it matters:**
- Very high event throughput (thousands of events/second across orgs)
- Need for per-org or per-session topic subscriptions to reduce noise
- Complex fan-out patterns (e.g., Slack integration needing filtered event streams)

**Redis solution:** Redis Pub/Sub or Redis Streams for durable, filtered event delivery.

**Alternative without Redis:** Current PostgreSQL NOTIFY + polling fallback is fine for the foreseeable scale.

### 4. Session/Ephemeral State — Not Needed

SSE connections are stateless (resume via `since_id`). No session affinity required. No shopping-cart-style ephemeral state. Workers are stateless task executors. **No use case here.**

### 5. Distributed Locking — Not Needed

`SKIP LOCKED` handles task claiming. Advisory locks handle migrations. No external coordination needed. **PostgreSQL covers this completely.**

## Recommendation

### Don't Add Redis Yet

**Rationale:**
1. **Operational simplicity** — One fewer dependency to deploy, monitor, back up, secure. This was the explicit design philosophy (dismissed Temporal for the same reason).
2. **Per-instance rate limiting is adequate** — Auth-only rate limits with small instance counts. The blast radius of N× overshoot on login attempts is minimal.
3. **In-process caching suffices** — `moka` or `mini-moka` crate gives TTL-based LRU per instance. Config data tolerates seconds of staleness.
4. **NOTIFY works** — Current event volumes don't stress the 8KB limit or listener reliability.

### Add Redis When (Trigger Conditions)

| Trigger | Signal | Action |
|---------|--------|--------|
| Per-org API rate limits ship | Product decision to enforce tenant quotas | Add Redis for distributed token bucket |
| Per-org LLM budgets ship | Need cross-worker TPM/RPM coordination | Add Redis for sliding-window counters |
| `GetTurnContext` becomes hot | P99 > 50ms or PG CPU > 60% from config reads | Add Redis read-through cache |
| 10+ control-plane instances | Per-instance rate limiting error exceeds 10× | Add Redis for coordinated limits |
| Event throughput > 1000/s sustained | PgListener reconnect gaps cause visible latency | Evaluate Redis Streams |

### If/When We Add It

**Prefer Valkey over Redis.** Valkey is the community fork (Linux Foundation) after Redis relicensed (SSPL). API-compatible, actively maintained, no licensing concerns.

**Start with a single use case** (likely distributed rate limiting), not a wholesale migration. Keep PostgreSQL as source of truth. Redis is a cache/coordinator, never the primary store.

**Crate options:**
- `fred` — Full-featured async Redis client, cluster support, Lua scripting
- `redis-rs` — Widely used, lighter weight
- `deadpool-redis` — Connection pooling

## What NOT to Use Redis For

- **Primary data storage** — PostgreSQL handles this. Don't duplicate.
- **Task queue replacement** — `SKIP LOCKED` works. Adding a Redis queue means two queue systems.
- **Distributed locks** — PostgreSQL advisory locks are sufficient and transactional.
- **Session storage** — Sessions are stateless. No server-side session store needed.
