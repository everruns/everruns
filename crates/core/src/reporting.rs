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

/// Half-open time window applied to the dataset's primary timestamp column
/// during a report query. `from` is inclusive, `to` is exclusive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportTimeRange {
    /// Start of the window (RFC 3339, inclusive).
    #[cfg_attr(feature = "openapi", schema(example = "2026-04-24T00:00:00Z"))]
    pub from: DateTime<Utc>,
    /// End of the window (RFC 3339, exclusive).
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-24T00:00:00Z"))]
    pub to: DateTime<Utc>,
}

/// Semantic query a caller submits to the reporting layer. The backend
/// compiles this to its native query language, scopes it to the calling
/// org, and returns a `ReportResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportQuery {
    /// Dataset name to query (see `GET /v1/reports/catalog` for the list of available datasets).
    pub dataset: String,
    /// Time window for the query. The dataset selects which timestamp column the range applies to.
    pub time_range: ReportTimeRange,
    /// Columns to group by. Empty list returns one aggregate row.
    #[serde(default)]
    pub dimensions: Vec<String>,
    /// Aggregations to compute (count, sum, avg, etc.). Empty list returns row counts only.
    #[serde(default)]
    pub measures: Vec<String>,
    /// Predicate filters applied before aggregation.
    #[serde(default)]
    pub filters: Vec<ReportFilter>,
    /// Sort spec applied after aggregation. Empty list yields unspecified order.
    #[serde(default)]
    pub order_by: Vec<ReportOrderBy>,
    /// Maximum number of rows to return (defaults to 100).
    #[serde(default = "default_report_limit")]
    pub limit: u32,
}

fn default_report_limit() -> u32 {
    100
}

/// One predicate filter applied to the dataset before aggregation.
/// Combined with other filters via logical AND.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportFilter {
    /// Field to filter on. Must be a filter field exposed by the dataset (see `filter_fields` in the catalog).
    #[cfg_attr(feature = "openapi", schema(example = "status"))]
    pub field: String,
    /// Comparison operator. Determines the expected shape of `value`
    /// (scalar for `eq`/`neq`/`gt`/`gte`/`lt`/`lte`, array for `in`).
    pub op: ReportFilterOp,
    /// Comparison value. Type depends on `op`: a scalar for `eq`/`neq`/`gt`/`gte`/`lt`/`lte`,
    /// an array for `in`. Example for `op = in`: `["completed", "failed"]`.
    pub value: Value,
}

/// Comparison operator used in a `ReportFilter`. The `In` variant takes a
/// JSON array as its value; all others take a scalar.
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

/// One sort clause applied to the aggregated result. Either `dimension`
/// OR `measure` is set (mutually exclusive), never both.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportOrderBy {
    /// Dimension name to sort by. Mutually exclusive with `measure`.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = "org_id"))]
    pub dimension: Option<String>,
    /// Measure name to sort by. Mutually exclusive with `dimension`.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = "session_count"))]
    pub measure: Option<String>,
    /// Sort direction (`asc` or `desc`). Defaults to `asc`.
    #[serde(default = "default_order_direction")]
    pub direction: ReportOrderDirection,
}

fn default_order_direction() -> ReportOrderDirection {
    ReportOrderDirection::Asc
}

/// Sort direction for a `ReportOrderBy` clause.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ReportOrderDirection {
    Asc,
    Desc,
}

/// Materialized result of a report query — column metadata, rows, and the
/// freshness of the underlying data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportResult {
    /// Timestamp the underlying data was materialized (RFC 3339). Useful as
    /// an "as of" footer when rendering — distinct from when the query ran.
    pub as_of: DateTime<Utc>,
    /// How stale the data is relative to the server's wall clock at query
    /// time, in milliseconds. `None` when freshness can't be determined
    /// (e.g. backends that don't track projector lag).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness_lag_ms: Option<i64>,
    /// Column metadata in the same order as the entries of each row in
    /// `rows`. Use the `kind` field to tell dimensions from measures.
    pub columns: Vec<ReportColumn>,
    /// Result rows. Each row is a JSON object keyed by column name; cell
    /// types match the underlying dataset (numbers for measures, strings
    /// or numbers for dimensions). Length is capped by `ReportQuery.limit`.
    pub rows: Vec<Value>,
}

/// One column header in a `ReportResult`. The ordered `columns` list
/// declares the key set of each row in `rows`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReportColumn {
    /// Column name as it appears in `rows`.
    #[cfg_attr(feature = "openapi", schema(example = "session_count"))]
    pub name: String,
    /// Whether this column is a grouping `dimension` or an aggregate
    /// `measure` — the same distinction made on `ReportQuery`.
    pub kind: ReportColumnKind,
}

/// Whether a `ReportColumn` is a grouping dimension or an aggregate measure.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ReportColumnKind {
    Dimension,
    Measure,
}

/// Listing of every dataset the reporting layer can answer queries over.
/// Returned from `GET /v1/reports/catalog`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DatasetCatalog {
    /// All datasets the caller has access to, in stable alphabetical order.
    pub datasets: Vec<DatasetCatalogEntry>,
}

/// A single dataset entry in the reporting catalog — the set of dimensions,
/// measures, and filter fields the dataset exposes to query authors.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DatasetCatalogEntry {
    /// Dataset identifier as passed to `ReportQuery.dataset`.
    pub name: String,
    /// Dimensions available to group by.
    pub dimensions: Vec<String>,
    /// Measures available to aggregate.
    pub measures: Vec<String>,
    /// Fields valid as the `field` of a `ReportFilter`.
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
