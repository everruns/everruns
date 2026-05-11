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
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub query: ReportQuery,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<SavedReportDashboardMetadata>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SavedReportDashboardMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSavedReportRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub query: ReportQuery,
    #[serde(default)]
    pub dashboard: Option<SavedReportDashboardMetadata>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateSavedReportRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(value_type = Option<String>, nullable = true)]
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

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ExportReportQueryRequest {
    pub query: ReportQuery,
    #[serde(default = "default_export_format")]
    pub format: ReportExportFormat,
}

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
    pub id: Uuid,
    pub org_id: i64,
    pub source_type: String,
    pub source_id: String,
    pub attempts: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectorRunResult {
    pub claimed: usize,
    pub completed: usize,
    pub failed: usize,
}
