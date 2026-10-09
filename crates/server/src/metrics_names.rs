//! Well-known metric names (all prefixed `everruns_`) to avoid typos across
//! the codebase.
//!
//! Decision: lives at the crate root, not under `crate::api`, so domains,
//! storage and services can emit metrics without importing the HTTP layer.
//! `crate::api::prometheus::names` re-exports this module.

// === Gauges (from DB via MetricsCollector bridge) ===
// Each replica reads the same DB and emits the same values. Prometheus
// keeps separate series per `instance`. Queries for cluster-level values
// should aggregate: `max without(instance) (everruns_workflows_running)`.
pub const WORKFLOWS_RUNNING: &str = "everruns_workflows_running";
pub const WORKFLOWS_PENDING: &str = "everruns_workflows_pending";
pub const TASKS_PENDING: &str = "everruns_tasks_pending";
pub const TASKS_CLAIMED: &str = "everruns_tasks_claimed";
pub const WORKERS_ACTIVE: &str = "everruns_workers_active";
pub const LOAD_RATIO: &str = "everruns_load_ratio";
pub const DLQ_SIZE: &str = "everruns_dlq_size";
/// Current connections opened by an sqlx pool. Label: pool.
pub const DATABASE_POOL_SIZE: &str = "everruns_database_pool_size";
/// Configured connection ceiling for an sqlx pool. Label: pool.
pub const DATABASE_POOL_MAX_SIZE: &str = "everruns_database_pool_max_size";
/// Open connections currently idle in an sqlx pool. Label: pool.
pub const DATABASE_POOL_IDLE: &str = "everruns_database_pool_idle";
/// Open connections currently checked out from an sqlx pool. Label: pool.
pub const DATABASE_POOL_IN_USE: &str = "everruns_database_pool_in_use";
// DB cumulative totals as gauges (not _total — these are global state, not
// per-instance counters). These are monotonically increasing in normal
// operation. Use delta() in PromQL for rate-like queries on gauges.
pub const TASKS_COMPLETED: &str = "everruns_tasks_completed";
pub const TASKS_FAILED: &str = "everruns_tasks_failed";
pub const TASKS_STARTED: &str = "everruns_tasks_started";
pub const WORKFLOWS_COMPLETED: &str = "everruns_workflows_completed";
pub const WORKFLOWS_FAILED: &str = "everruns_workflows_failed";
pub const WORKFLOWS_STARTED: &str = "everruns_workflows_started";

// === Counters (from local events — per-instance, no double-counting) ===
pub const HTTP_REQUESTS_TOTAL: &str = "everruns_http_requests_total";
pub const LLM_REQUESTS_TOTAL: &str = "everruns_llm_requests_total";
pub const TOOL_EXECUTIONS_TOTAL: &str = "everruns_tool_executions_total";
/// Generations by normalized finish reason. Labels: provider, model,
/// finish_reason (`stop`, `tool_calls`, `length`, `content_filter`, ...).
pub const LLM_FINISH_REASON_TOTAL: &str = "everruns_llm_finish_reason_total";
/// Tool calls a driver discarded because the response was cut off or
/// rejected. Labels: provider, model, reason (the finish reason).
pub const LLM_TOOL_CALLS_DROPPED_TOTAL: &str = "everruns_llm_tool_calls_dropped_total";
/// Tool calls run from a truncated response; their own arguments are
/// complete. Labels: provider, model, reason (the finish reason).
pub const LLM_TOOL_CALLS_TRUNCATED_EXECUTED_TOTAL: &str =
    "everruns_llm_tool_calls_truncated_executed_total";
/// Generations the output-truncation gate acted on. Labels: provider,
/// model, action (`retried` | `failed`).
pub const LLM_TRUNCATION_GATE_TOTAL: &str = "everruns_llm_truncation_gate_total";
/// Provider retries before a generation succeeded. Label: provider.
pub const LLM_RETRIES_TOTAL: &str = "everruns_llm_retries_total";
/// Counter for every domain Command invocation across HTTP, MCP and
/// gRPC ExecuteCommand. Labels: name, category, status (ok |
/// bad_request | unprocessable | forbidden | not_found | conflict |
/// rate_limited | unavailable | internal).
pub const COMMANDS_TOTAL: &str = "everruns_commands_total";
/// Entity history rows that could not be written (see
/// `domains::change_history`): the change rolled back with them inside a
/// command transaction, or stands without its entry outside one. Should
/// stay at zero.
pub const ENTITY_HISTORY_WRITE_FAILURES: &str = "everruns_entity_history_write_failures_total";
/// Agent-made changes recorded without a reason, while the org does not
/// require one. Label: entity_kind.
pub const ENTITY_CHANGES_WITHOUT_REASON: &str = "everruns_entity_changes_without_reason_total";
/// Queries that ran on their own connection while the task had a command
/// transaction open, so they commit independently of it (see
/// `storage::transaction`). Label: path (raw_pool | savepoint_open).
/// Nonzero values point at paths not yet converted to the transaction.
pub const DB_QUERIES_OUTSIDE_TRANSACTION: &str =
    "everruns_db_queries_outside_command_transaction_total";
/// Messages refused because their org reached `ORG_MAX_ACTIVE_TURNS`, a
/// protective limit. The org is in the log line; nonzero means an org hit it.
pub const ORG_ACTIVE_TURN_CAP_REJECTIONS_TOTAL: &str =
    "everruns_org_active_turn_cap_rejections_total";

/// Orphaned blob objects deleted by the object-storage GC sweep (objects
/// present in the bucket with no live sidecar pointer, older than the grace
/// period). Per-instance counter.
pub const BLOB_GC_ORPHANS_DELETED_TOTAL: &str = "everruns_blob_gc_orphans_deleted_total";
/// Bytes reclaimed by the object-storage GC sweep. Per-instance counter.
pub const BLOB_GC_BYTES_RECLAIMED_TOTAL: &str = "everruns_blob_gc_bytes_reclaimed_total";

// === Histograms (from local observations — per-instance) ===
pub const HTTP_REQUEST_DURATION: &str = "everruns_http_request_duration_seconds";
pub const LLM_REQUEST_DURATION: &str = "everruns_llm_request_duration_seconds";
pub const TOOL_EXECUTION_DURATION: &str = "everruns_tool_execution_duration_seconds";
/// Total backoff (Retry-After or computed) a retried generation waited.
/// Label: provider.
pub const LLM_RETRY_WAIT_DURATION: &str = "everruns_llm_retry_wait_seconds";
/// Wall-clock duration of every domain Command invocation. Same labels
/// as `COMMANDS_TOTAL`.
pub const COMMAND_DURATION: &str = "everruns_command_duration_seconds";
/// Input message persisted to `turn.started`: enqueue plus worker pickup.
pub const TURN_PICKUP_DURATION: &str = "everruns_turn_pickup_seconds";
/// Input message to the first streamed token of the turn's first LLM call.
pub const TURN_FIRST_TOKEN_DURATION: &str = "everruns_turn_first_token_seconds";
/// Hand-off between durable turn phases (queue wait). Label: phase
/// (reason | act), the phase being started.
pub const TURN_PHASE_GAP_DURATION: &str = "everruns_turn_phase_gap_seconds";
/// Turn wall-clock not spent in the LLM or tools. Label: outcome.
pub const TURN_OVERHEAD_DURATION: &str = "everruns_turn_overhead_seconds";
