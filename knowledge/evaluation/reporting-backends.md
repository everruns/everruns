---
type: Specification
title: "Reporting Backends Evaluation"
description: "Phase 3 reference evaluation."
tags:
  - everruns
  - evaluation
---
# Reporting Backends Evaluation

Reference evaluation of future analytical backends for the reporting abstraction
defined in [`knowledge/evaluation/reporting.md`](reporting.md). Phase 3 only, not part of the
phase 1 PostgreSQL foundation or phase 2 capability/saved-report layers.

## Goals

- Evaluate StarRocks and DuckDB-over-object-storage as future implementations of
  `ReportingProjectionSink` and `ReportingQueryBackend`.
- Confirm both can be added without changing canonical operational tables, the
  semantic query API, source keys, idempotent projection contracts, or
  org-scoped facts.
- Surface the deployment, cost, and operational tradeoffs that drive backend
  choice per environment.

## Non-Goals

- Recommending a default phase 3 backend before the PostgreSQL foundation and
  capability/saved-report layers are proven in production.
- Specifying a migration plan from PostgreSQL facts. Migration is replay from
  canonical sources via `reporting_outbox`, not bulk-copy of fact rows.
- Replacing `events`, `usage_journal`, `usage_ledger`, or `budgets` as sources
  of truth. Reporting facts remain derived and replayable.

## Compatibility Boundary

Both candidates must implement the same contract as the PostgreSQL backend:

| Surface | Contract |
|---|---|
| `ReportingProjectionSink::upsert_facts` | Idempotent upsert by source key per fact dataset |
| `ReportingProjectionSink::supersede_source` | Replace facts for a source on replay |
| `ReportingQueryBackend::query` | Execute a `ReportQuery` under a `ReportScope` |
| Source keys | `event:<id>`, `session:<id>:turn:<id>`, `usage_ledger:<id>`, etc. (see [`knowledge/evaluation/reporting.md`](reporting.md#idempotency)) |
| Org isolation | `org_id` on every fact row; query compiler injects `org_id = scope.org_id` |
| Time bucketing | `created_at` and `time_bucket_day` are first-class dimensions |
| Dataset shape | Flat, denormalized, with display-name snapshots |

A phase 3 backend MUST NOT require:

- Changes to `reporting_outbox` row shape.
- Changes to canonical operational tables.
- Backend-specific filter operators or measure semantics.
- Cross-org or cross-tenant facts. Single-tenant slicing is mandatory.

If a candidate cannot meet these constraints, it is dismissed, not adopted with
a per-backend semantic API.

## StarRocks

### Profile

- Massively parallel processing OLAP database, MySQL-protocol compatible.
- Designed for always-on grouped aggregation with high concurrency and
  low-latency interactive dashboards.
- Native columnar storage with primary-key tables that support upsert.

### Why It Fits

- Primary-key tables map cleanly to fact source keys: `(org_id, source_key)`
  primary key gives idempotent upsert with replace-on-conflict semantics.
- Distributed execution scales grouped aggregations on `fact_llm_generation`,
  `fact_tool_call`, and `fact_capability_usage` without per-org sharding logic
  in the application.
- MySQL-protocol query layer permits a SQL builder very similar to the
  PostgreSQL backend, so the semantic query plan compiles to similar shapes.
- Bitmap indexes and bucketed distribution speed up high-cardinality dimensions
  like `model`, `provider`, `tool_name`, and `capability_id`.

### Risks And Constraints

- Operational complexity: FE/BE/CN cluster, separate metadata service, more
  moving parts than a managed PostgreSQL.
- Backup, snapshot, and disaster recovery story differs from PostgreSQL and
  must be designed before adoption.
- Memory-heavy workloads. Concurrency limits and per-query memory caps must be
  configured deliberately for shared multi-tenant deployments.
- Org isolation remains an application-layer concern. StarRocks does not
  provide per-row tenant boundaries beyond what the query compiler enforces;
  this is acceptable but must be tested under load.
- Direct ingestion via Stream Load or Routine Load needs an adapter that wraps
  the existing projector output. Bulk replay must remain idempotent under
  retries.

### Operational Profile

- Best fit: hosted multi-tenant deployments with always-on dashboards and many
  concurrent viewers.
- Not a fit: single-org self-hosted deployments where PostgreSQL already meets
  reporting load.
- Cost driver: cluster compute and memory. Storage cost is comparable to a
  similarly-sized PostgreSQL.

### Phase 3 Adoption Test

Before promoting StarRocks past reference status:

1. Implement `ReportingProjectionSink` for StarRocks Stream Load in a separate
   crate that depends only on the contract from `everruns-core`.
2. Implement `ReportingQueryBackend` using a SQL builder shared with the
   PostgreSQL backend where practical.
3. Replay a representative org's history through `reporting_outbox` and confirm
   parity of every dataset's measures and dimensions against the PostgreSQL
   backend within freshness tolerance.
4. Stress-test concurrency: at least 50 concurrent viewers per org running the
   heaviest dashboards with p95 query latency under 1 second, sustained for at
   least 30 minutes, and verify per-query memory caps prevent noisy-neighbor
   effects across orgs. Adjust the concurrency target upward to match real
   dashboard usage at promotion time, but do not promote below this floor.

## DuckDB Over S3-Compatible Object Storage

### Profile

- In-process analytical engine with strong Parquet support and httpfs/s3
  extensions.
- Designed for embedded batch analytics, ad-hoc queries, and exports rather
  than high-concurrency dashboards.

### Why It Fits

- Parquet partitions map naturally to org-scoped facts:
  ```text
  s3://reports/fact_tool_call/org_id=1/day=2026-05-06/*.parquet
  s3://reports/fact_llm_generation/org_id=1/day=2026-05-06/*.parquet
  s3://reports/fact_budget_posting/org_id=1/day=2026-05-06/*.parquet
  ```
- The `org_id=...` partition prefix is an optimization that lets predicate
  pushdown prune unrelated org partitions before scan, but it is not the
  isolation boundary. Tenant scope is still enforced by the query compiler
  always injecting `org_id = scope.org_id` and by storage/IAM policies that
  prevent cross-prefix access; the partition layout simply makes that scope
  cheap to evaluate.
- Cheap long-term retention. Cold facts can stay in object storage at low cost
  with tiered storage policies independent of operational PostgreSQL retention.
- Excellent fit for export and offline analytics. `EXPORT DATABASE` and
  `COPY (SELECT ...) TO 's3://...'` align with phase 2 export requirements.

### Risks And Constraints

- Concurrency is per-process. Multi-user dashboards need a fan-out pool of
  DuckDB workers or a coordinator. Not suited as the primary backend for an
  always-on dashboard tier without additional infrastructure.
- Idempotency requires careful Parquet file management. Source-key upsert is
  not native to Parquet; the projector must write per-source files (or per-day
  files keyed by source key) and the query backend must deduplicate on read,
  or a compaction job must rewrite files.
- Schema evolution depends on Parquet logical types. Adding measures or
  dimensions requires either backward-compatible reads or a one-shot rewrite.
- Object-storage latency for small files is significant. Projection batches
  must be sized to amortize per-object request cost.

### Operational Profile

- Best fit: lightweight self-hosted deployments, batch reports, scheduled
  exports, and cheap long-term retention.
- Not a fit: high-concurrency multi-tenant dashboards with sub-second latency.
- Cost driver: object-storage requests and egress. Compute is cheap because
  DuckDB runs in-process.

### Phase 3 Adoption Test

Before promoting DuckDB past reference status:

1. Implement `ReportingProjectionSink` that batches facts to per-day Parquet
   files keyed by `(org_id, day, source_key)` or per-source-key files inside
   the day partition.
2. Implement a compaction job that rewrites partitions to deduplicate by
   source key and to coalesce small files.
3. Implement `ReportingQueryBackend` that opens DuckDB with the httpfs/s3
   extension, applies `org_id` and time-range filters as partition pruning, and
   runs the semantic query plan.
4. Confirm parity against PostgreSQL on a representative org and freshness
   tolerance under realistic projection cadence.
5. Verify export shapes and retention policies align with phase 2 export
   requirements.

## Comparison Summary

| Dimension | StarRocks | DuckDB + Object Storage |
|---|---|---|
| Primary use case | Always-on multi-user dashboards | Batch reports, exports, cheap retention |
| Concurrency model | Cluster with high concurrency | Per-process; needs fan-out for multi-user |
| Idempotent upsert | Native via primary-key tables | Application-managed via partitioning + compaction |
| Org isolation | Application-layer scope injection | Application-layer scope injection + path partition |
| Cold retention cost | Same tier as hot | Cheap object storage |
| Operational footprint | Cluster (FE/BE/CN) | Object storage + compaction job |
| Schema evolution | Online schema change | Parquet-compatible additions |
| Best for | Hosted multi-tenant deployments | Self-hosted, batch, exports, long retention |

## Recommendation

Phase 3 stays reference-only until the PostgreSQL foundation and capability /
saved-report layers (phase 1 and phase 2) ship and run in production with
representative load. After that:

- Adopt **StarRocks** when reporting load profile is dominated by always-on
  multi-user dashboards with high concurrency, sub-second latency, and grouped
  aggregations across high-cardinality dimensions.
- Adopt **DuckDB over S3-compatible object storage** when reporting load
  profile is dominated by batch reports, exports, scheduled aggregates, and
  long-tail retention; or when the primary deployment target is a single-org
  self-hosted environment.

Both backends MUST plug into the existing `ReportingProjectionSink` and
`ReportingQueryBackend` traits without changes to canonical tables, the
semantic query API, or org isolation rules.

## Open Questions

- Whether StarRocks Routine Load or Stream Load is preferable for the projector
  ingestion path under our retry semantics.
- Whether DuckDB's per-process concurrency limit is best addressed by a
  read-only worker pool or by promoting a coordinator service.
- How retention policies on DuckDB Parquet partitions interact with privacy
  deletion requirements for display-name snapshots.
- Whether a hybrid deployment (PostgreSQL hot, DuckDB cold) is worth the
  operational cost vs. a single-backend deployment per environment.
