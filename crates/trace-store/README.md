# Optional DataFusion trace store

An optional companion for backfilling retained Everruns events and running
DataFusion queries alongside PostgreSQL. It uses immutable Parquet indexes,
separate payload objects and a PostgreSQL metastore. The normal server and
Framework do not link DataFusion. Existing product reads, execution recovery,
SSE, Trace UI, billing and reporting retain their PostgreSQL paths.

The [architecture](../../knowledge/operations/trace-storage.md) owns the design,
consistency boundaries and promotion criteria. This guide owns exact commands.
Cloud I/O uses the platform's patched object-store backend. DataFusion receives
validated in-memory Arrow tables and has no cloud client. Its older transitive
object-store generation keeps cloud features disabled; its S3/XML stack is absent.

## Off by default

```bash
cargo build -p everruns-trace-store
# No database, S3 credentials, directory or listener needed:
target/debug/everruns-trace-store run
# {"enabled":false}
```

`TRACE_STORE_ENABLED` defaults to `false`. Disabled `run` exits before any
storage/network initialization. Installing the binary or enabling file/image
S3 offload never enables this companion. Conversely, trace storage does not
change `STORAGE_BLOB_BACKEND`. Explicit operator commands such as `backfill`
are deliberate operations and require storage configuration.

Before enabling the companion, run `everruns-trace-store migrate-catalog`. This
applies only its two additive catalog migrations, transactionally, using a separate
`trace_store_schema` checksum ledger. It leaves `_sqlx_migrations` unchanged, so
the currently deployed platform can still restart. Future platform adoption
applies the same idempotent DDL through the normal migrator. The companion
cannot run arbitrary platform migrations. Give it canonical read access plus
trace-metastore writes. PostgreSQL row locks additionally require column-level
UPDATE permission on `sessions` (for example `UPDATE(id)`), or equivalent owner
privileges; source event tables need SELECT only. The code performs no canonical
DML, but this qualification connection is not a PostgreSQL read-only transaction.
It is a trusted operator tool, not a public query API;
`--org-id` is the internal DB tenant id and event/session ids are raw UUIDs.

## Backfill and parallel run

```bash
export DATABASE_URL='postgres://...'
# Local development placement uses the same storage contract as S3.
export TRACE_STORE_DIRECTORY='./work/trace-objects'

# Create fixed per-session targets and copy a bounded first pass.
target/debug/everruns-trace-store backfill --org-id 1 --max-passes 1
# Inspect and resume the returned run UUID. Repeating a pass is idempotent.
target/debug/everruns-trace-store backfill-status --org-id 1 --run-id "$RUN_UUID"
target/debug/everruns-trace-store backfill --org-id 1 --resume "$RUN_UUID" --max-passes 100

# A continuously supervised companion resumes its latest backfill, catches up
# new events, and runs rotating synthetic shadow comparisons.
TRACE_STORE_ENABLED=true TRACE_STORE_ORG_IDS=1 \
  target/debug/everruns-trace-store run

# Explicit all-organizations authority, including new orgs. Do not set ORG_IDS.
unset TRACE_STORE_ORG_IDS
TRACE_STORE_ENABLED=true TRACE_STORE_ALL_ORGS=true TRACE_STORE_MAX_ORGS=5 \
  target/debug/everruns-trace-store run
# Global payload-free progress for an all-org operator:
target/debug/everruns-trace-store status-all
```

All-org mode persists a rotating organization cursor, discovers only organizations
with event history, and processes bounded batches. Backlog/storage/unhealthy-check
gauges cover the whole store, preserving failures outside the current batch.
The per-org finite backfill remains resumable; new orgs join subsequent sweeps.

For S3, unset `TRACE_STORE_DIRECTORY` and set `TRACE_STORE_BUCKET`. The SDK
reads `AWS_DEFAULT_REGION`, `AWS_ENDPOINT` (optional S3-compatible HTTPS endpoint),
and its AWS credential configuration. Keep the bucket private, encrypted and
placed near compute. Use dedicated permissions and prefix. Local protocol tests
use explicit localhost HTTP; cloud endpoints must keep TLS verification enabled.

Backfill snapshots eligible sessions and their committed heads. New appends and
new sessions do not extend that run; continuous catch-up handles them afterward.
It reads both `events` and `archived_events`, deduplicates archival overlap by
canonical sequence/identity, rejects conflicting identities, and advances across
valid gaps. Missing retained history cannot be recovered: shadow reports it as
`missing_source`, not a verified empty match.

Each pass is bounded. Failures are recorded with a fixed code and retried on later
passes without starving other targets. Deleted/tombstoned sessions are skipped.
`pending`, `copied`, `skipped` and `failed` remain distinct; only zero pending and
zero failures establish run completion. `exported_this_pass` counts that pass's
actual copied rows. Progress survives process restarts; run identity is scoped to
placement and org.

## Configuration

| Setting | Default | Effect |
| --- | --- | --- |
| `TRACE_STORE_ENABLED` | `false` | Enables `run` only when explicitly true |
| `TRACE_STORE_ORG_IDS` | empty | Explicit org allowlist, mutually exclusive with all-org mode |
| `TRACE_STORE_ALL_ORGS` | `false` | Explicit all-org authority, including future history |
| `TRACE_STORE_MAX_ORGS` | `5` | Per-cycle all-org discovery batch; 1..100 |
| `TRACE_STORE_DIRECTORY` | unset | Local object root; conflicts with bucket |
| `TRACE_STORE_BUCKET` | unset | S3 placement; required unless a directory is selected |
| `TRACE_STORE_NAME` | `default` | Stable metastore placement identity |
| `TRACE_STORE_PREFIX` | `traces` | Dedicated, non-overlapping object namespace |
| `TRACE_STORE_METRICS_ADDR` | `127.0.0.1:9465` | Private observer endpoint |
| `TRACE_STORE_POLL_SECONDS` | `5` | Delay between cycles; 1..3600 |
| `TRACE_STORE_MAX_SESSIONS` | `5` | Per-org session batch; 1..100 |
| `TRACE_STORE_INDEX_CACHE_BYTES` | `67108864` | Decoded immutable index cache budget; zero disables caching |

A placement name identifies the same physical bucket/root for its lifetime.
Registration rejects overlapping prefixes and name/prefix changes; it cannot
detect a name silently pointed at a different physical bucket. Reserve its prefix:
GC reclaims every unreferenced object there. Use a new name and prefix for relocation.

The standalone [Compose profile](compose.yaml) starts no companion by default.
It uses a read-only non-root image, CPU/memory limits, no capabilities, and a
loopback-only published metrics port. Its cloud environment variables deliberately
use `TRACE_STORE_*` credentials rather than the application's file/image settings.
Choose an immutable reviewed image digest; the unpublished placeholder is not a
release. Build the image from the repository root:

```bash
docker build -f crates/trace-store/Dockerfile -t YOUR_TRACE_IMAGE .
# Under a managed build proxy, also pass its CA as --secret id=proxy_ca,src=...
# Select the profile only after configuring its explicit enablement and scope.
docker compose -f crates/trace-store/compose.yaml --profile trace-store up -d
```

Stopping this process/profile restores the off state without changing primary
service configuration. Ctrl+C interrupts the standalone process; checkpoints and
publication use the platform's cancellation-safe transaction helpers.

## Inspect and compare

Ordinary session/sequence pages use an ordered index fast path, stopping at the
requested page plus one continuation row. Event detail uses an index locator
directly. Filtered and cross-session queries use a shared DataFusion execution
context over query-local Arrow tables. There are no shared per-query table/store
registrations. Cold indexes require one bounded range GET and checksum-verified
decode. Warm reads reuse immutable decoded indexes; payloads are never cached.

Every read still locks/selects current live manifests and checks tenant scope.
Cache keys include immutable manifest identity, object sizes and checksum;
new exports and compaction cannot reuse stale entries. Tombstones/deleted source
sessions are checked before cache access. Explicit trace deletion also evicts the
instance's cached indexes for that session. Other readers still revalidate live
manifests. The cache uses a byte budget and an entry ceiling, LRU
eviction and a serialized decoder that remains bounded after cancellation.
Decoded query inputs and results have admission budgets in addition to the
DataFusion operator pool. These are not whole-process RSS guarantees.

Each shadow window now compares PostgreSQL with the initial serving read, a
repeat of that identical read, a forced DataFusion read of the identical window,
and a sampled payload lookup. All variants must match. `ReadProfile` records the
actual engine, cold/warm/mixed/empty cache state and fixed stages; a second call
is not assumed warm if eviction/compaction or an oversized index caused misses.

Prometheus `read_seconds` histograms label `kind`, `engine` and `cache`;
`stage_seconds` label `kind`, `engine` and `stage`. Names have the usual
`everruns_trace_store_` prefix. Stages separate catalog, index I/O, decoding,
planning, execution, payload I/O and payload checksum/decoding. Unexecuted stages
do not add zero samples.
Cache bytes/entries/evictions/hits/misses, index bytes read and observed Linux
resident/peak-resident memory and aggregate process CPU seconds are also exported
without tenant/object labels. CPU time uses the kernel's clock-tick frequency;
resource metrics are omitted when unavailable rather than fabricated as zero.
The legacy end-to-end summary kind is `trace_summary`; it includes whichever
serving engine the query actually used. Use the new engine-labelled histograms
for analytical versus serving comparisons and histogram quantile estimates.

The local latency sample prints exact sample p50/p95/p99 without payloads:

```bash
DATABASE_URL=postgres://... cargo test -p everruns-trace-store \
  --features postgres-tests serving_latency_sample -- --nocapture
```

That fixture is not a production benchmark. Production comparisons must retain
identical predicates/sequence boundaries and distinguish cold from warm reads,
source load, cache pressure, resource limits and ongoing backfill.

```bash
target/debug/everruns-trace-store status --org-id 1
target/debug/everruns-trace-store query --org-id 1 --session-id "$SESSION_UUID" --limit 100
target/debug/everruns-trace-store get --org-id 1 --session-id "$SESSION_UUID" --event-id "$EVENT_UUID"
target/debug/everruns-trace-store shadow --org-id 1 --session-id "$SESSION_UUID"
```

Queries return summaries, `has_more` and candidate-segment count. Results sort by
session UUID then canonical sequence; session pagination passes the last returned
sequence as `--after-sequence`. Type and turn filters are typed literals. Time
ranges are half-open occurrence-time ranges. Separate pages are fresh snapshots,
not canonical multi-page replay. Org-wide continuation is not implemented.

Shadow reads pin both stores to the shared exported head, then prune the physical
query to the sampled page plus its continuation row. It rotates a sequence cursor
through long histories. It compares summaries and samples a complete envelope;
raw JSON, metadata and tags must agree. `lagging` can have true parity for the
copied prefix; events above the shared head are not mismatches. Errors/budget
overflows and absent retained history cannot be counted as matches. Reports
preserve source/export heads, sampled window, outcome and independent timings.

This is a generated workload over real retained history, **not mirrored HTTP
traffic**. Timings are individual samples, not production benchmark conclusions.
Payloads are sampled, not exhaustively compared. Latest evidence is stored in
`trace_store_shadow_checks`; it contains no payload/diff/error text. Outstanding
mismatch/error/missing-source checks remain unhealthy until replaced or the
session is deleted. The raw `get` result is privileged and must not bypass
existing public event disclosure rules in a future integration.

## Monitor

```bash
curl -fsS http://127.0.0.1:9465/metrics
curl -fsS http://127.0.0.1:9465/healthz
curl -fsS http://127.0.0.1:9465/readyz
target/debug/everruns-trace-store probe
```

`healthz` establishes liveness; `readyz` requires a recent successful cycle and no
outstanding mismatches/errors. Backfill completion is a separate status. Prometheus
counters/histograms reset on process restart; durable latest comparison evidence
and backfill checkpoints survive. Metrics never label by org/session, object path,
prompt or credential. Capture the private endpoint with your own monitoring system;
exposing it does not configure a remote scraper or alert channel.

Useful PromQL:

```promql
sum(rate(everruns_trace_store_comparisons_total{outcome="mismatch"}[5m]))
histogram_quantile(0.95, sum by (le, kind) (rate(everruns_trace_store_query_seconds_bucket[5m])))
max(everruns_trace_store_lag_sequences)
```

Track cycle errors/age, copied rows, pending sessions, sequence lag (includes valid
sequence gaps), backfill failures, outcome counters, source/DataFusion summary and
payload latency histograms, and published index/payload bytes. Published bytes do
not measure billable object requests, egress or total system cost. Compare source
DB load and resource usage separately during a real burn-in.

## Maintenance and limits

```bash
target/debug/everruns-trace-store compact --org-id 1 --session-id "$SESSION_UUID"
target/debug/everruns-trace-store delete --org-id 1 --session-id "$SESSION_UUID"
target/debug/everruns-trace-store gc --limit 1000
# Continue GC with --after <next_cursor> until it returns null.
```

Compaction atomically swaps bounded adjacent groups. Deletion hides the analytical
copy and blocks re-export before physical GC; canonical rows remain. Source-session
deletion also hides its copies. GC excludes active readers/unpublished uploads with
a store-wide advisory lock. Catalog tombstones/retired manifests remain for audit. Object deletion does not
purge historical versions in a versioned bucket; their lifecycle and backup
erasure need a separate policy before retention/deletion promotion.
Maintenance is explicit in this version; no automatic TTL or canonical deletion.

Default budgets are 2,000 rows/32 MiB per export, 8 MiB per event, 20,000 rows/128 MiB
per compaction, 256 candidate segments/64 MiB indexes per query, 1,000 returned rows,
a 128 MiB DataFusion operator pool with no spill files, and a 15-second query deadline.
The pool is not an OS RSS limit. Export/compaction/GC have 60-second deadlines, source
SQL a 20-second statement timeout and 5-second lock timeout. The process holds at
most four PostgreSQL connections and performs org work sequentially. GC retains
at most 1,001 object metadata entries and rejects listings beyond 100,000 objects.
Its coarse lock and listing ceiling need redesign before large-scale promotion.

Budget overflow is explicit. Canonical cutover, hot/cold replay, full-text/JSON
indexes, analytical aggregate APIs and distributed scheduling remain outside this
qualification phase. PostgreSQL continues storing every authoritative event.

## Verify

```bash
cargo test -p everruns-trace-store
PORT_PREFIX=519 just start-infra
DATABASE_URL=postgres://everruns:everruns@localhost:51932/everruns \
  cargo test -p everruns-trace-store --features postgres-tests
# Optional local SeaweedFS protocol fixture, as configured in CI:
TEST_TRACE_S3_ENDPOINT=http://127.0.0.1:18333 \
DATABASE_URL=postgres://everruns:everruns@localhost:51932/everruns \
  cargo test -p everruns-trace-store --features postgres-tests,s3-tests
PORT_PREFIX=519 just stop-infra
cargo clippy -p everruns-trace-store --all-targets --features postgres-tests,s3-tests -- -D warnings
```

The S3 test accepts localhost endpoints and fixed test credentials only. Lifecycle
tests execute both real catalog migrations and cover archival overlap, fixed
backfill targets, resumption, gaps/conflicts, tenant isolation, live catch-up,
rotating comparisons, corruption, privacy, disabled mode, failures and maintenance.
