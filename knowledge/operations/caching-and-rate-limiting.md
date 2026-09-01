---
type: Specification
title: "Caching and Rate Limiting"
description: "Two-tier caching and rate-limiting strategy: PostgreSQL as the sole store of record, Valkey for cross-instance limits, moka for in-process hot paths."
tags:
  - everruns
  - operations
  - caching
  - rate-limiting
---
# Caching and Rate Limiting

## Abstract

Everruns keeps exactly one stateful dependency of record: PostgreSQL. Two caching
tiers sit beside it and neither is allowed to become a second source of truth.
Valkey (Redis-compatible, Linux Foundation fork) coordinates rate limits across
control-plane instances; `moka` caches hot-path reads inside a single process.
Both tiers are optional in the sense that the system stays correct without them,
only slower or less strict.

## Design Decisions

- **PostgreSQL stays the only store of record.** Every cache entry must be
  reconstructible from the database. This is what makes cache loss a latency
  event rather than a data-loss event, and it is why the rules under "What
  Valkey Is Not For" exist.
- **Rate limiting is a control-plane concern.** Workers are stateless gRPC
  clients; they hold no budget of their own. Per-worker LLM limiting (TPM/RPM)
  is therefore mediated: a worker asks the control-plane for a token before
  calling a provider. The extra round-trip is negligible against provider
  latency, and it keeps one authority over the budget instead of N.
- **Valkey is optional, and its absence degrades strictness, not
  availability.** With `VALKEY_URL` unset the limiter falls back to an
  in-process `governor` instance, so N instances grant N× the configured
  budget. That is an accepted trade for single-instance and local runs; shared
  sliding-window counters are what a multi-instance deployment buys by setting
  the variable. See `crates/server/src/auth/rate_limit.rs` for the dual-backend
  limiter and `crates/server/src/valkey.rs` for the sliding-window Lua script.
- **Rate limiting fails open.** A Valkey error allows the request. An outage of
  an optional dependency must not take down authentication; availability wins
  over strictness here, and the residual risk is recorded as TM-DOS-009
  (network exposure) and TM-DOS-010 (fail-open) in the
  [threat model](../security/threat-model.md).
- **In-process caching is for repeated reads inside one turn or one short
  window.** Auth material, resolved providers and models, decrypted data
  encryption keys, and capability metadata are read many times per turn and
  change rarely. They are cached with a TTL rather than invalidated eagerly:
  bounded staleness is acceptable for all of them, and a TTL cannot leak a
  missed invalidation. Cache sites live next to their readers, not in a shared
  cache module.
- **Caches are per-instance and unshared.** Nothing in the in-process tier is
  replicated. Two instances may hold different values for the same key within
  the TTL window; no behavior may depend on them agreeing.

## What Valkey Is Not For

Recorded because each of these was considered and rejected, and each would
introduce a second system that has to be operated and reasoned about:

| Use | Why not | What is used instead |
|---|---|---|
| Primary data storage | It is a cache; loss must stay recoverable | PostgreSQL |
| Task queue | A second queue system with its own failure modes | `SELECT ... FOR UPDATE SKIP LOCKED` |
| Distributed locks | No added guarantee over what the database already gives | PostgreSQL advisory locks |
| Session storage | Sessions are resumable from the event log (`since_id`), so there is no session state to store | Event replay |

## When to Extend

The current footprint is deliberate. Each row below names the signal that would
justify widening it, so the decision is made against evidence rather than
anticipation.

| Trigger | Signal | Action |
|---|---|---|
| Per-org API rate limits | Product decision to sell tenant quotas | Extend the Valkey limiter beyond the auth surface |
| Per-org LLM budgets | Cross-worker TPM/RPM coordination required | Valkey sliding window in front of provider calls |
| `GetTurnContext` on the hot path | P99 > 50 ms or PostgreSQL CPU > 60% | Valkey read-through cache |
| Event throughput above ~1000/s | `PgListener` reconnect gaps become visible | Evaluate Valkey Streams |

## Where the Details Live

| Detail | Source of truth |
|---|---|
| Current limits, windows, and which surfaces are limited | `crates/server/src/auth/rate_limit.rs` |
| Sliding-window algorithm and client configuration | `crates/server/src/valkey.rs` |
| `VALKEY_URL` and related configuration | [`docs/sre/environment-variables.md`](../../docs/sre/environment-variables.md) |
| Local and production process startup | `scripts/lib/infra.sh`, `scripts/lib/services.sh` |
| Crate versions and features (`fred`, `moka`, `governor`) | [Dependency Surface](../project/dependency-surface.md), `Cargo.toml` |

## See also

- [Architecture](../foundations/architecture.md), multi-instance safety and the stateful dependency set
- [Production Deployment](production-deployment.md), what a deployment runs and what is optional
- [Authentication](../security/authentication.md), the abuse limits the auth surface enforces
- [Threat Model](../security/threat-model.md), TM-DOS-009 and TM-DOS-010
