# Caching & Distributed Rate Limiting

## Abstract

Caching and rate limiting strategy for everruns. PostgreSQL is the sole stateful dependency for persistence. Valkey (Redis-compatible, Linux Foundation fork) provides distributed rate limiting. In-process caching (`moka` crate) handles hot-path deduplication.

## Valkey: Distributed Rate Limiting

Valkey is optional infrastructure for coordinating rate limits across 10+ control-plane instances.

- **Crate**: `fred` v10 (`enable-rustls-ring`, `i-scripts` features)
- **Module**: `crates/server/src/valkey.rs`
- **Env var**: `VALKEY_URL` (e.g., `redis://localhost:6379`). See `docs/sre/environment-variables.md`.
- **Algorithm**: Sliding-window counter via atomic Lua script (sorted set per key)
- **Fail-open**: On Valkey errors, requests are allowed (availability > strictness)
- **Threat model**: TM-DOS-009 (network exposure), TM-DOS-010 (fail-open design)

### Dual-Backend Rate Limiting

`AuthRateLimiter` (`crates/server/src/auth/rate_limit.rs`) supports two backends:

| Backend | When | Coordination |
|---------|------|-------------|
| In-memory (governor) | `VALKEY_URL` not set | Per-instance only; N instances = N× budget |
| Valkey | `VALKEY_URL` set | Shared sliding-window counters across all instances |

Current rate limits (auth endpoints only):

| Endpoint | Limit | Window |
|----------|-------|--------|
| Login | 10/min | 60s |
| Register | 5/min | 60s |
| Refresh | 30/min | 60s |

### Deployment Topology

Workers are stateless gRPC clients. All rate limiting happens in the control-plane:

```
Workers ──gRPC──► Control-Plane ──► PostgreSQL
                       │
                       └──► Valkey (rate limits, optional)
```

For per-worker LLM rate limiting (TPM/RPM), the control-plane mediates: workers request a rate-limit token via gRPC before calling the LLM provider. Adds ~1ms round-trip (negligible vs 500ms+ LLM latency).

### Infrastructure

- `scripts/lib/infra.sh` — Starts PostgreSQL and Valkey as native processes
- `scripts/lib/services.sh` — Auto-starts Valkey in `start-all` and `start-production`

## In-Process Caching Opportunities

Independent of Valkey. Use `moka` crate (async, TTL + max-size eviction). Tracked in Linear EVE-47 through EVE-51.

| Priority | Target | Fix | Linear |
|----------|--------|-----|--------|
| Critical | `get_agent_capabilities()` called 4×/turn | Load once, pass through | EVE-47 |
| High | API key auth — 4 sequential DB queries | Cache `key_hash → AuthUser`, 5-min TTL | EVE-48 |
| High | LLM model/provider resolution | Cache `(org_id, model_id) → ResolvedModel`, 1-hour TTL | EVE-49 |
| Medium | Encryption key lookups | Cache decrypted DEKs by `org_id` | EVE-50 |
| Medium | Active skills list | Cache `org_id → Vec<Skill>`, 5-min TTL | EVE-51 |

Already optimized: feature flags (startup), MCP tool cache (1-hour TTL), capability registry (in-memory).

## Future Trigger Conditions

| Trigger | Signal | Action |
|---------|--------|--------|
| Per-org API rate limits | Product decision for tenant quotas | Extend Valkey to API endpoints |
| Per-org LLM budgets | Cross-worker TPM/RPM coordination | Valkey sliding-window for LLM |
| `GetTurnContext` hot path | P99 > 50ms or PG CPU > 60% | Valkey read-through cache |
| Event throughput > 1000/s | PgListener reconnect gaps | Evaluate Valkey Streams |

## What NOT to Use Valkey For

- **Primary data storage** — PostgreSQL handles this
- **Task queue** — `SKIP LOCKED` works; don't add a second queue system
- **Distributed locks** — PostgreSQL advisory locks are sufficient
- **Session storage** — Sessions are stateless (resume via `since_id`)
