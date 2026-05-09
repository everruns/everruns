pub use everruns_core::reporting::{
    DatasetCatalog, DatasetCatalogEntry, ReportColumn, ReportColumnKind, ReportFilter,
    ReportFilterOp, ReportOrderBy, ReportOrderDirection, ReportQuery, ReportResult, ReportScope,
    ReportTimeRange,
};

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ProjectorRunResult {
    pub claimed: usize,
    pub completed: usize,
    pub failed: usize,
}
