use crate::api::common::deserialize_nullable_update_field;
use chrono::{DateTime, Utc};
pub use everruns_core::reporting::{
    DatasetCatalog, DatasetCatalogEntry, ReportColumn, ReportColumnKind, ReportFilter,
    ReportFilterOp, ReportOrderBy, ReportOrderDirection, ReportQuery, ReportResult, ReportScope,
    ReportTimeRange,
};
use everruns_durable::UpdateField;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// A user-saved report definition — a named, persistable wrapper around a
/// `ReportQuery` with optional dashboard placement metadata.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SavedReport {
    /// UUID of the saved report.
    pub id: Uuid,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    /// The query this report executes when run or exported. Same shape as the
    /// `body` of `POST /v1/reports/query` — see `ReportQuery` for the field
    /// breakdown.
    pub query: ReportQuery,
    /// Optional dashboard placement metadata. `None` means the report is
    /// "library-only" and not pinned to a dashboard layout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<SavedReportDashboardMetadata>,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

/// Dashboard placement metadata attached to a `SavedReport`. Captures how
/// and where to render the report in the operator dashboard UI.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SavedReportDashboardMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Human-readable title. Safe to render in user-facing messages.
    pub title: Option<String>,
    /// Dashboard section/group this report belongs to (free-form bucket name
    /// like `"Operations"` or `"Finance"`). Used to cluster related reports
    /// in the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// Rendering hint for the UI (e.g. `"line"`, `"bar"`, `"table"`,
    /// `"big_number"`). Free-form string — the server doesn't enforce a
    /// closed set so new chart types can roll out client-side first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart_type: Option<String>,
    /// Sort key within `section`. Lower values render first. `None` means
    /// "place at the end".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
}

/// Request body for the `create_saved_report` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSavedReportRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "Weekly active agents — last 30 days")]
    pub name: String,
    /// Human-readable description. Safe to render in user-facing messages.
    #[serde(default)]
    #[schema(
        example = "Rolling 30-day count of agents with at least one session per day, grouped by org."
    )]
    pub description: Option<String>,
    /// The query this report executes when run or exported. See `ReportQuery`
    /// for the full field breakdown.
    pub query: ReportQuery,
    /// Optional dashboard placement metadata. Omit to create a library-only
    /// report that isn't pinned to any dashboard layout.
    #[serde(default)]
    pub dashboard: Option<SavedReportDashboardMetadata>,
}

/// Request body for the `update_saved_report` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateSavedReportRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    #[serde(default)]
    #[schema(example = "Weekly active agents — last 60 days")]
    pub name: Option<String>,
    /// Human-readable description. Safe to render in user-facing messages.
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true, example = "Rolling 60-day window; widened from 30d after the Q3 product launch.")]
    pub description: UpdateField<String>,
    /// Replace the saved report's query wholesale. Omit to keep the existing
    /// query; send a new `ReportQuery` to swap it.
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = ReportQuery, nullable = false)]
    pub query: UpdateField<ReportQuery>,
    /// Replace dashboard placement metadata. Omit to keep current placement;
    /// send `null` to detach the report from its dashboard; send an object
    /// to overwrite the placement.
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<SavedReportDashboardMetadata>, nullable = true)]
    pub dashboard: UpdateField<SavedReportDashboardMetadata>,
}

/// Output format for a report export. `Csv` emits a header row plus one
/// row per result; `Json` emits an envelope with the same shape as
/// `ReportResult`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportExportFormat {
    Csv,
    Json,
}

/// Request body for the `export_report_query` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ExportReportQueryRequest {
    /// Ad-hoc query to materialize and export. Same shape as the body of
    /// `POST /v1/reports/query` — see `ReportQuery` for the field breakdown.
    pub query: ReportQuery,
    /// Export format. Defaults to `csv` when omitted.
    #[serde(default = "default_export_format")]
    pub format: ReportExportFormat,
}

/// Request body for the `export_saved_report` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ExportSavedReportRequest {
    /// Export format. Defaults to `csv` when omitted.
    #[serde(default = "default_export_format")]
    pub format: ReportExportFormat,
}

fn default_export_format() -> ReportExportFormat {
    ReportExportFormat::Csv
}

/// Serialized export of a report's data, ready to stream to a caller as a
/// download. Carries the rendered payload plus the MIME/filename metadata
/// a client needs to save it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportExport {
    /// Export format (currently `csv`).
    pub format: ReportExportFormat,
    /// Suggested filename for the download (includes the extension matching `format`).
    pub filename: String,
    /// MIME type matching `format` (`text/csv` for CSV exports).
    pub content_type: String,
    /// Serialized payload as a UTF-8 string. Caller streams this to the client.
    pub content: String,
    /// Timestamp the underlying data was materialized (RFC 3339). Useful for "as of" footers.
    pub as_of: DateTime<Utc>,
    /// How stale the data is relative to `now()`, in milliseconds. `None` when freshness can't be determined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness_lag_ms: Option<i64>,
}

/// Point-in-time health snapshot of the reporting layer — projector
/// freshness plus outbox processing health. Returned from
/// `GET /v1/reports/admin/diagnostics`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportingDiagnostics {
    /// Server-side wall-clock timestamp when this snapshot was assembled
    /// (RFC 3339). Use as the "as-of" for any displayed lag metrics.
    pub generated_at: DateTime<Utc>,
    /// Per-dataset projector freshness. One entry per active reporting
    /// dataset; missing datasets mean the projector hasn't produced any
    /// rows yet.
    pub projector_lag: Vec<DatasetProjectorLag>,
    /// Reporting outbox health — counts of pending/processing/failed/
    /// completed rows plus a sample of the most recent failures.
    pub outbox: ReportingOutboxDiagnostics,
}

/// Per-dataset projector freshness telemetry. One entry per active dataset
/// the reporting projector is materializing.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DatasetProjectorLag {
    /// Dataset name (matches `ReportQuery.dataset`).
    pub dataset: String,
    /// Wall-clock timestamp of the newest fact the projector has
    /// materialized for this dataset (RFC 3339). `None` if the projector
    /// hasn't produced any rows yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_projected_at: Option<DateTime<Utc>>,
    /// Gap between `latest_projected_at` and the diagnostic's
    /// `generated_at`, in milliseconds. `None` when freshness can't be
    /// determined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness_lag_ms: Option<i64>,
}

/// Aggregate health of the reporting outbox — the queue of source rows
/// waiting to be projected into fact tables.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportingOutboxDiagnostics {
    /// Outbox rows waiting to be claimed by a projector.
    pub pending: i64,
    /// Outbox rows currently being processed.
    pub processing: i64,
    /// Outbox rows that have failed (exceeded retry limit).
    pub failed: i64,
    /// Outbox rows that have completed processing successfully.
    pub completed: i64,
    /// Timestamp of the oldest `pending` row (RFC 3339). `None` if no rows are pending.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_pending_at: Option<DateTime<Utc>>,
    /// Sample of the most recent failed rows for operator inspection.
    pub failed_rows: Vec<FailedReportingOutboxRow>,
}

/// One failed reporting-outbox row, surfaced in
/// `ReportingOutboxDiagnostics.failed_rows` so operators can triage
/// projector failures without dropping to SQL.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FailedReportingOutboxRow {
    /// Outbox row UUID.
    pub id: Uuid,
    /// Owning organization's internal numeric id.
    pub org_id: i64,
    /// Discriminator for the outbox source (`event`, `session`, `llm_generation`, `usage_ledger`).
    pub source_type: String,
    /// Source-specific row identifier (event id, session id, etc.).
    pub source_id: String,
    /// Number of processing attempts made before this row was marked failed.
    pub attempts: i32,
    /// Most recent error message from a processing attempt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Timestamp when this row was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

/// Outcome of one projector run — how many outbox rows it claimed,
/// completed, and failed. Returned by manual `POST /v1/reports/projector/run`
/// calls and useful for backfill scripting.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectorRunResult {
    /// Number of outbox rows claimed by this run.
    pub claimed: usize,
    /// Number of claimed rows that completed successfully.
    pub completed: usize,
    /// Number of claimed rows that failed and will be retried (or moved to `failed` after retry limit).
    pub failed: usize,
}

/// Request body for the `reporting_backfill` operation — enqueues source
/// rows into the reporting outbox for the projector to re-materialize.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ReportingBackfillRequest {
    /// Maximum number of outbox rows to enqueue across all source types. Defaults to 1000.
    #[serde(default = "default_backfill_limit")]
    #[schema(example = 5000)]
    pub limit: i64,
}

fn default_backfill_limit() -> i64 {
    1_000
}

/// Result of a `reporting_backfill` call — per-source counts of outbox rows
/// enqueued for re-projection.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct ReportingBackfillResult {
    /// Total number of outbox rows enqueued across all source types.
    pub enqueued: i64,
    /// Number of `event` outbox rows enqueued.
    pub events: i64,
    /// Number of `session` outbox rows enqueued.
    pub sessions: i64,
    /// Number of `llm_generation` outbox rows enqueued.
    pub llm_generations: i64,
    /// Number of `usage_ledger` outbox rows enqueued.
    pub usage_ledger: i64,
}
