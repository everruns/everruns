//! Backend-neutral reporting contract.
//!
//! Reporting facts are derived, org-scoped analytical data. Callers submit a
//! constrained semantic query; backend implementations compile that shape to
//! their own storage/query language and must inject tenant scope themselves.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Caller;

#[derive(Debug, Clone)]
pub struct ReportScope {
    pub org_id: i64,
    pub caller: Caller,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportTimeRange {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportQuery {
    pub dataset: String,
    pub time_range: ReportTimeRange,
    #[serde(default)]
    pub dimensions: Vec<String>,
    #[serde(default)]
    pub measures: Vec<String>,
    #[serde(default)]
    pub filters: Vec<ReportFilter>,
    #[serde(default)]
    pub order_by: Vec<ReportOrderBy>,
    #[serde(default = "default_report_limit")]
    pub limit: u32,
}

fn default_report_limit() -> u32 {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportFilter {
    pub field: String,
    pub op: ReportFilterOp,
    pub value: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ReportFilterOp {
    Eq,
    Neq,
    In,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportOrderBy {
    #[serde(default)]
    pub dimension: Option<String>,
    #[serde(default)]
    pub measure: Option<String>,
    #[serde(default = "default_order_direction")]
    pub direction: ReportOrderDirection,
}

fn default_order_direction() -> ReportOrderDirection {
    ReportOrderDirection::Asc
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ReportOrderDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportResult {
    pub as_of: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness_lag_ms: Option<i64>,
    pub columns: Vec<ReportColumn>,
    pub rows: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportColumn {
    pub name: String,
    pub kind: ReportColumnKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ReportColumnKind {
    Dimension,
    Measure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DatasetCatalog {
    pub datasets: Vec<DatasetCatalogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DatasetCatalogEntry {
    pub name: String,
    pub dimensions: Vec<String>,
    pub measures: Vec<String>,
    pub filter_fields: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SourceKey {
    pub source_type: String,
    pub source_id: String,
}

#[derive(Debug, Clone)]
pub struct FactBatch {
    pub records: Vec<FactRecord>,
}

#[derive(Debug, Clone)]
pub struct FactRecord {
    pub dataset: String,
    pub org_id: i64,
    pub source_key: String,
    pub values: Value,
}

#[async_trait]
pub trait ReportingProjectionSink: Send + Sync {
    async fn upsert_facts(&self, batch: FactBatch) -> anyhow::Result<()>;
    async fn supersede_source(&self, source: SourceKey) -> anyhow::Result<()>;
}

#[async_trait]
pub trait ReportingQueryBackend: Send + Sync {
    async fn query(&self, scope: ReportScope, query: ReportQuery) -> anyhow::Result<ReportResult>;
}
