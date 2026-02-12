# Scalability Analysis

Status: Analysis
Date: 2025-02-12
Scope: Identify bottlenecks blocking Everruns from scaling beyond ~100 concurrent users / ~50 concurrent workers

## Executive Summary

Everruns has strong architectural foundations for horizontal scaling: stateless control plane, PostgreSQL-backed shared state, gRPC worker isolation, and SKIP LOCKED task distribution. However, several implementation gaps will cause failures before reaching the documented 1000-worker target. This document identifies those gaps and proposes a prioritized roadmap.

## Methodology

Analysis based on reading production source code in `crates/server/`, `crates/worker/`, `crates/durable/`, `crates/core/`, SQL migrations, and existing specs/threat-model.

---

## Critical Issues (P0)

### 1. Database Connection Pool Unconfigured

**Location:** `crates/server/src/storage/repositories.rs:24-26`

```rust
pub async fn from_url(database_url: &str) -> Result<Self> {
    let pool = PgPool::connect(database_url).await?;
    Ok(Self { pool })
}
```

**Problem:** `PgPool::connect()` uses sqlx defaults (~10 max connections). At scale, gRPC handlers serving 100+ concurrent workers will exhaust the pool instantly. Every DB-backed operation queues behind 10 connections.

**Impact:** Request latency spikes, timeouts, cascading failures under moderate load.

**Fix:** Use `PgPoolOptions` with configurable `max_connections`, `min_connections`, `acquire_timeout`, and `idle_timeout`. Expose via `DATABASE_POOL_MAX` env var. Recommended starting point: `max_connections = 50` per control plane instance, with `acquire_timeout = 5s`.

**Effort:** Low
**Risk if unfixed:** System unusable beyond ~20 concurrent workers

---

### 2. Unbounded Session Message Loading

**Location:** gRPC `get_turn_context()` and `event_service.list_message_events(session_id)`

**Problem:** `GetTurnContext` loads ALL message events for a session into memory with no limit. A session with 10,000 messages (~50MB) is loaded, converted to proto, and transmitted over gRPC in a single response. With 100 concurrent workers fetching turn context, that is 5GB of memory allocation.

**Impact:** OOM kills on control plane. gRPC timeouts for large sessions.

**Fix (two-phase):**
1. **Immediate:** Add a `LIMIT` + keyset pagination to message event queries. Workers should receive the most recent N messages (e.g., 200) plus a summary/context window.
2. **Follow-up:** Implement context window management — truncate old messages, summarize, or use sliding window. This aligns with how LLM context windows work anyway.

**Effort:** Medium
**Risk if unfixed:** OOM crashes with long-running sessions

---

### 3. LLM Provider Rate Limiting Missing

**Location:** No rate limiting layer exists between workers and LLM providers.

**Problem:** Workers call LLM APIs directly with no concurrency control. 100 workers simultaneously calling OpenAI with 2k tokens each = 200k tokens/request burst, easily exceeding typical rate limits (500k TPM, 200 RPM). The `durable_circuit_breaker_state` table exists in the migration but comments indicate it is not yet integrated.

**Impact:** Mass 429 errors from providers, wasted tokens on retries, degraded user experience.

**Fix:** Add a `LlmRateLimiter` middleware in the worker's LLM driver call path:
- Per-provider token bucket (configurable TPM/RPM limits)
- Semaphore-based concurrency limit (e.g., max 50 concurrent LLM calls per provider)
- Integrate existing circuit breaker table for automatic backoff on sustained 429s

**Effort:** High
**Risk if unfixed:** Provider bans, unpredictable latency, cost overruns

---

### 4. SSE Connection Exhaustion (Open Threat TM-DOS-003)

**Location:** `crates/server/src/api/events.rs` — SSE endpoint

**Problem:** No limit on concurrent SSE connections per user, per session, or globally. Each SSE stream holds a Tokio task and polls the database every 100-500ms. 10,000 open streams = 50,000+ queries/second from polling alone.

**Impact:** Database overwhelmed by polling queries. Memory exhaustion from accumulated Tokio tasks.

**Fix:**
- Global SSE connection limit (e.g., 10,000)
- Per-session limit (e.g., 5 concurrent SSE connections)
- Per-user limit (e.g., 50)
- Replace polling with `pg_notify` (already used for durable task queue — proven pattern in this codebase)

**Effort:** Medium
**Risk if unfixed:** Single user can DoS the control plane

---

## High Priority Issues (P1)

### 5. Event Table Unbounded Growth

**Location:** `events` table — append-only, no archival, no partitioning

**Problem:** Events are immutable and never deleted (enforced by trigger). No retention policy, no archival strategy. At scale:
- 10k users × 100 sessions × 1000 events = 1B rows
- Single table with B-tree index on `(session_id, sequence)` degrades
- JSONB data column adds 15-30% storage overhead

**Fix (phased):**
1. **Phase 1:** Add configurable `EVENT_RETENTION_DAYS` with a background job that archives old events to cold storage and removes from hot table (relaxing the immutability trigger for the archival process).
2. **Phase 2:** Partition `events` table by `session_id` hash (PostgreSQL declarative partitioning). The migration already has a comment about this being future work.
3. **Phase 3:** Time-based partitioning for oldest data, enabling `DROP PARTITION` for fast bulk deletes.

**Effort:** High
**Risk if unfixed:** Gradual query degradation, storage costs, backup time growth

---

### 6. SSE Polling Instead of Push Notifications

**Location:** `crates/server/src/api/events.rs` — stream uses `event_service.list()` in a poll loop

**Problem:** SSE streams poll the database at 100ms-500ms intervals. The durable task queue already uses `pg_notify` for push-based distribution with P50 ~4ms latency. Events don't use this pattern, meaning:
- Unnecessary DB load (N streams × poll_rate queries/sec)
- Higher latency for event delivery (up to 500ms vs ~4ms)

**Fix:** Apply the same `pg_notify`/`PgListener` pattern used in `crates/durable/` to the events system. Notify on `INSERT INTO events`, SSE streams subscribe via `PgListener`, fall back to 10s poll on disconnect.

**Effort:** Medium
**Risk if unfixed:** DB load scales linearly with SSE connections

---

### 7. gRPC Authentication Missing (Open Threat TM-DURABLE-002)

**Location:** gRPC service on port 9001 — no auth layer

**Problem:** Worker-to-control-plane gRPC is unauthenticated. Any process that can reach port 9001 can claim tasks, emit events, and read session data. In a multi-tenant deployment, this is a critical security gap that also affects scalability — you can't safely expose the control plane across network boundaries.

**Fix:** mTLS for worker authentication, or bearer token auth with per-worker tokens. This unblocks deploying workers in separate network zones / clusters.

**Effort:** Medium
**Risk if unfixed:** Cannot safely scale workers across network boundaries

---

## Medium Priority Issues (P2)

### 8. GetTurnContext N+1 MCP Queries

**Location:** `build_mcp_tool_definitions()` in gRPC service

**Problem:** For each MCP capability on an agent, a separate DB query fetches tool definitions. Agent with 5 MCP servers = 5 queries per `GetTurnContext` call. At 100 workers, that's 500 extra queries per turn cycle.

**Fix:** Batch MCP tool loading into a single query (`WHERE mcp_server_id IN (...)`) or cache MCP tool definitions with TTL (tools change infrequently).

**Effort:** Low
**Risk if unfixed:** Unnecessary DB load, increased turn latency

---

### 9. Durable Engine Missing Org Scoping

**Location:** `durable_*` tables lack `org_id` column

**Problem:** All durable workflows, tasks, and schedules are in a single global namespace. No per-org isolation, quotas, or access control. This blocks multi-tenant scaling.

**Fix:** Add `org_id` to all durable tables, enforce in queries. Add per-org limits on concurrent workflows and task queue depth.

**Effort:** Medium (migration + query changes)
**Risk if unfixed:** Cannot offer durable execution to multiple tenants safely

---

### 10. No Backpressure on Task Queue Creation (Open Threat TM-DOS-006)

**Location:** Task creation path has no rate limiting

**Problem:** A single agent can flood the durable task queue with unlimited pending tasks. Workers must process them all. No per-org or per-agent task creation limits.

**Fix:** Per-org task creation rate limit. Max pending tasks per workflow. Queue depth monitoring with alerts.

**Effort:** Low-Medium
**Risk if unfixed:** Noisy neighbor degrades all tenants

---

## Recommended Roadmap

### Phase 1: Unblock 100-Worker Scale (immediate)

| # | Item | Effort |
|---|------|--------|
| 1 | Configure DB connection pool (`PgPoolOptions`) | Low |
| 2 | Add LIMIT to message event queries in GetTurnContext | Low |
| 3 | SSE connection limits (global + per-session + per-user) | Medium |
| 4 | Batch MCP tool loading | Low |

### Phase 2: Unblock 500-Worker Scale

| # | Item | Effort |
|---|------|--------|
| 5 | LLM provider rate limiting + circuit breaker integration | High |
| 6 | SSE push notifications via pg_notify | Medium |
| 7 | gRPC mTLS or bearer token auth | Medium |
| 8 | Task queue backpressure limits | Low |

### Phase 3: Unblock 1000+ Worker Scale

| # | Item | Effort |
|---|------|--------|
| 9 | Event table partitioning | High |
| 10 | Event archival + retention policy | High |
| 11 | Durable engine org scoping | Medium |
| 12 | Context window management for long sessions | High |

---

## What Already Works Well

- **SKIP LOCKED task claiming** — proven pattern, benchmarked at 10k tasks/sec
- **Stateless control plane** — no process-local state, ready for load balancer
- **gRPC worker isolation** — workers have no direct DB access
- **Advisory locks for migrations** — multi-instance safe
- **pg_notify for durable tasks** — P50 ~4ms push latency
- **UUID v7 for all IDs** — time-ordered, B-tree friendly
- **Partial indexes on task queue** — `WHERE status = 'pending'` avoids scanning completed tasks

## References

- `specs/architecture.md` — System architecture
- `specs/durable-execution-engine.md` — Durable engine design
- `specs/threat-model.md` — Open threats TM-DOS-003, TM-DOS-006, TM-DURABLE-002
- `specs/events-contract.md` — SSE event format
- `specs/multitenancy.md` — Org-based isolation model
