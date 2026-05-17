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

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SavedReport {
    /// Prefixed public identifier (see `specs/id-schema.md`).
    pub id: Uuid,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    pub query: ReportQuery,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<SavedReportDashboardMetadata>,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SavedReportDashboardMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Human-readable title. Safe to render in user-facing messages.
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
}

/// Request body for the `create_saved_report` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSavedReportRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(default)]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    pub query: ReportQuery,
    #[serde(default)]
    pub dashboard: Option<SavedReportDashboardMetadata>,
}

/// Request body for the `update_saved_report` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateSavedReportRequest {
    #[serde(default)]
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: UpdateField<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = ReportQuery, nullable = false)]
    pub query: UpdateField<ReportQuery>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<SavedReportDashboardMetadata>, nullable = true)]
    pub dashboard: UpdateField<SavedReportDashboardMetadata>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportExportFormat {
    Csv,
    Json,
}

/// Request body for the `export_report_query` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ExportReportQueryRequest {
    pub query: ReportQuery,
    #[serde(default = "default_export_format")]
    pub format: ReportExportFormat,
}

/// Request body for the `export_saved_report` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ExportSavedReportRequest {
    #[serde(default = "default_export_format")]
    pub format: ReportExportFormat,
}

fn default_export_format() -> ReportExportFormat {
    ReportExportFormat::Csv
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportExport {
    pub format: ReportExportFormat,
    pub filename: String,
    pub content_type: String,
    pub content: String,
    pub as_of: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness_lag_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportingDiagnostics {
    pub generated_at: DateTime<Utc>,
    pub projector_lag: Vec<DatasetProjectorLag>,
    pub outbox: ReportingOutboxDiagnostics,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DatasetProjectorLag {
    pub dataset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_projected_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness_lag_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportingOutboxDiagnostics {
    pub pending: i64,
    pub processing: i64,
    pub failed: i64,
    pub completed: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_pending_at: Option<DateTime<Utc>>,
    pub failed_rows: Vec<FailedReportingOutboxRow>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FailedReportingOutboxRow {
    /// Prefixed public identifier (see `specs/id-schema.md`).
    pub id: Uuid,
    /// Owning organization's prefixed public identifier.
    pub org_id: i64,
    pub source_type: String,
    pub source_id: String,
    pub attempts: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Timestamp when this resource was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectorRunResult {
    pub claimed: usize,
    pub completed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ReportingBackfillRequest {
    #[serde(default = "default_backfill_limit")]
    pub limit: i64,
}

fn default_backfill_limit() -> i64 {
    1_000
}

#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct ReportingBackfillResult {
    pub enqueued: i64,
    pub events: i64,
    pub sessions: i64,
    pub llm_generations: i64,
    pub usage_ledger: i64,
}
