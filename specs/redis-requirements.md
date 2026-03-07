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

## In-Process Caching Opportunities (No Redis Required)

Before reaching for Redis, several hot-path inefficiencies can be fixed with in-process caching. These are **independent of the Redis decision** and should be done regardless.

### Critical: Turn-Local Deduplication

`get_agent_capabilities()` is called **4 times per turn** across `get_agent()`, `build_tool_registry()`, `build_mcp_tool_definitions()`, and `agent_store::get_agent()` — all within a single `load_turn_context()` call chain (`direct_worker_adapters.rs`). Fix: load once, pass the result through.

### High: API Key Auth — 4 Sequential DB Queries Per Request

Every API-key-authenticated request runs 4 sequential queries in `auth/builtin.rs`: `get_api_key_by_hash()` → `get_user()` → `get_organization()` → `get_organization_member()`. These rarely change. Fix: in-process cache keyed on `key_hash` → `AuthUser` with 5-min TTL (`moka` crate).

### High: LLM Model/Provider Resolution

`llm_resolver.rs` queries provider + model config per LLM call. Providers/models change rarely. Fix: in-process cache `(org_id, model_id) → ResolvedModel` with 1-hour TTL, invalidated on provider/model update.

### Medium: Encryption Key Lookups

`storage/encryption.rs` decrypts the org encryption key per request in the auth flow. Fix: cache decrypted DEKs in memory keyed on `org_id`, invalidate on key rotation (rare).

### Medium: Active Skills List

`services/skill.rs` queries skills per capability listing. Fix: `(org_id → Vec<Skill>)` with 5-min TTL.

### Already Optimized

- **Feature flags** — loaded once at startup, served from memory
- **MCP tool cache** — 1-hour TTL with freshness checks
- **Capability registry** — in-memory built-in registry

### Recommended Crate

`moka` — async-compatible, TTL + max-size eviction, battle-tested. No Redis needed for any of the above.

### Impact on Redis Decision

If these in-process caches are implemented, the "cross-instance cache" Redis use case drops to **very low priority**. Each instance caches independently; config data tolerates seconds of staleness across instances. Redis for caching only becomes relevant if cache-miss thundering-herd on instance restart is a problem (unlikely with warm-up).

## Deployment Topology: Workers Don't Need Valkey

Workers are stateless gRPC clients. All rate limiting and caching happens in the control-plane:

```
Workers ──gRPC──► Control-Plane ──► PostgreSQL
                       │
                       └──► Valkey (rate limits only, when needed)
```

For per-worker LLM rate limiting (TPM/RPM), the control-plane mediates: workers request a rate-limit token via gRPC before calling the LLM provider. Adds ~1ms round-trip (negligible vs 500ms+ LLM latency). Keeps Valkey access centralized; workers need zero infrastructure beyond gRPC.

## Current State: Valkey Added for Distributed Rate Limiting

Valkey has been added as the first Redis-compatible dependency, specifically for distributed rate limiting across 10+ control-plane instances.

### What Was Done

- **Crate**: `fred` v10 with `enable-rustls-ring` and `i-scripts` features (Lua scripting for atomic rate limit operations)
- **Module**: `crates/server/src/valkey.rs` — `ValkeyClient` wrapper with sliding-window rate limiting via Lua script
- **Dual backend**: `AuthRateLimiter` supports both in-memory (governor) and Valkey backends. When `VALKEY_URL` is set, uses Valkey; otherwise falls back to per-instance in-memory limiting
- **Fail-open**: On Valkey errors, requests are allowed (availability > strictness)
- **Infrastructure**: Added to `local/docker-compose.yml`, `examples/docker-compose-full.yaml`, `.env.example`, `scripts/lib/services.sh`, and no-docker-setup scripts

### Remaining Trigger Conditions

| Trigger | Signal | Action |
|---------|--------|--------|
| Per-org API rate limits ship | Product decision to enforce tenant quotas | Extend Valkey rate limiting to API endpoints |
| Per-org LLM budgets ship | Need cross-worker TPM/RPM coordination | Add Valkey sliding-window counters for LLM |
| `GetTurnContext` becomes hot | P99 > 50ms or PG CPU > 60% from config reads | Add Valkey read-through cache |
| Event throughput > 1000/s sustained | PgListener reconnect gaps cause visible latency | Evaluate Valkey Streams |

## What NOT to Use Redis For

- **Primary data storage** — PostgreSQL handles this. Don't duplicate.
- **Task queue replacement** — `SKIP LOCKED` works. Adding a Redis queue means two queue systems.
- **Distributed locks** — PostgreSQL advisory locks are sufficient and transactional.
- **Session storage** — Sessions are stateless. No server-side session store needed.
