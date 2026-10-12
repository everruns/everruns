use crate::domains::reporting::record::{DatasetCatalog, ReportQuery, ReportResult, ReportScope};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::catalog;
use super::types::{
    CreateSavedReportRequest, ExportReportQueryRequest, ExportSavedReportRequest,
    ProjectorRunResult, ReportExport, ReportingBackfillRequest, ReportingBackfillResult,
    ReportingDiagnostics, SavedReport, UpdateSavedReportRequest,
};
use super::{REPORT_ADMIN, REPORT_MANAGE, REPORT_VIEW};
use crate::domains::common::{CliExample, CliRoute, Command, CommandError, Ctx, command};

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct RunReportQuery(pub ReportQuery);

#[command(
    name = "run_report_query",
    category = "reporting",
    description = "Run an org-scoped semantic reporting query.",
    method = "POST",
    path = "/v1/reports/query",
    policy = REPORT_VIEW,
    cli = CliRoute::new(&["reports", "query"], "run").with_examples(&[CliExample::new("Answer a quick question, such as sessions per status last month", "everruns reports query run --dataset sessions --time-range '{\"from\":\"2026-04-01T00:00:00Z\",\"to\":\"2026-05-01T00:00:00Z\"}' --dimensions status --measures session_count")]),
    read_only = true,
)]
impl Command for RunReportQuery {
    type Output = ReportResult;

    async fn execute(self, ctx: &Ctx) -> Result<ReportResult, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service
            .query(
                ReportScope {
                    org_id: ctx.org_id(),
                    caller: ctx.caller.clone(),
                },
                self.0,
            )
            .await
    }
}

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct GetReportCatalog;

#[command(
    name = "get_report_catalog",
    category = "reporting",
    description = "Return semantic reporting datasets, dimensions, measures, and filter fields.",
    method = "GET",
    path = "/v1/reports/catalog",
    policy = REPORT_VIEW,
    cli = CliRoute::new(&["reports", "catalog"], "get").with_examples(&[CliExample::new("Discover the datasets, dimensions and measures a query can use", "everruns reports catalog get")]),
)]
impl Command for GetReportCatalog {
    type Output = DatasetCatalog;

    async fn execute(self, _ctx: &Ctx) -> Result<DatasetCatalog, CommandError> {
        Ok(catalog::catalog())
    }
}

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct ListSavedReports;

#[command(
    name = "list_saved_reports",
    category = "reporting",
    description = "List org-scoped saved report definitions.",
    method = "GET",
    path = "/v1/reports/saved",
    policy = REPORT_VIEW,
    cli = CliRoute::new(&["reports", "saved"], "list").with_examples(&[CliExample::new("Find a saved report's id", "everruns reports saved list")]),
)]
impl Command for ListSavedReports {
    type Output = Vec<SavedReport>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SavedReport>, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service.list_saved_reports(ctx.org_id()).await
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetSavedReport {
    /// Saved report's prefixed public identifier.
    pub report_id: Uuid,
}

#[command(
    name = "get_saved_report",
    category = "reporting",
    description = "Get an org-scoped saved report definition.",
    method = "GET",
    path = "/v1/reports/saved/{report_id}",
    policy = REPORT_VIEW,
    cli = CliRoute::new(&["reports", "saved"], "get").with_examples(&[CliExample::new("Read a saved report's query before changing or running it", "everruns reports saved get --report-id 0190f8a2-7c1e-7d3a-9b2f-4e5d6c7b8a90")]),
)]
impl Command for GetSavedReport {
    type Output = SavedReport;

    async fn execute(self, ctx: &Ctx) -> Result<SavedReport, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service.get_saved_report(ctx.org_id(), self.report_id).await
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateSavedReport(pub CreateSavedReportRequest);

#[command(
    name = "create_saved_report",
    category = "reporting",
    description = "Create an org-scoped saved report definition.",
    method = "POST",
    path = "/v1/reports/saved",
    policy = REPORT_MANAGE,
    cli = CliRoute::new(&["reports", "saved"], "create").with_examples(&[CliExample::new("Save a query so the team can rerun it", "everruns reports saved create --name 'Sessions by status' --query '{\"dataset\":\"sessions\",\"time_range\":{\"from\":\"2026-04-01T00:00:00Z\",\"to\":\"2026-05-01T00:00:00Z\"},\"dimensions\":[\"status\"],\"measures\":[\"session_count\"],\"filters\":[],\"order_by\":[],\"limit\":100}' --reason 'Monthly review'")]),
)]
impl Command for CreateSavedReport {
    type Output = SavedReport;

    async fn execute(self, ctx: &Ctx) -> Result<SavedReport, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service.create_saved_report(ctx.org_id(), self.0).await
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateSavedReport {
    /// Saved report's prefixed public identifier.
    pub report_id: Uuid,
    pub request: UpdateSavedReportRequest,
}

#[command(
    name = "update_saved_report",
    category = "reporting",
    description = "Update an org-scoped saved report definition.",
    method = "PATCH",
    path = "/v1/reports/saved/{report_id}",
    policy = REPORT_MANAGE,
    cli = CliRoute::new(&["reports", "saved"], "update").with_examples(&[CliExample::new("Rename a saved report", "everruns reports saved update --report-id 0190f8a2-7c1e-7d3a-9b2f-4e5d6c7b8a90 --request '{\"name\":\"Weekly active agents\"}' --reason 'Clearer name'")]),
)]
impl Command for UpdateSavedReport {
    type Output = SavedReport;

    async fn execute(self, ctx: &Ctx) -> Result<SavedReport, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service
            .update_saved_report(ctx.org_id(), self.report_id, self.request)
            .await
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteSavedReport {
    /// Saved report's prefixed public identifier.
    pub report_id: Uuid,
}

#[command(
    name = "delete_saved_report",
    category = "reporting",
    description = "Delete an org-scoped saved report definition.",
    method = "DELETE",
    path = "/v1/reports/saved/{report_id}",
    policy = REPORT_MANAGE,
    cli = CliRoute::new(&["reports", "saved"], "delete").with_examples(&[CliExample::new("Delete a saved report nobody uses", "everruns reports saved delete --report-id 0190f8a2-7c1e-7d3a-9b2f-4e5d6c7b8a90 --reason 'Superseded by the weekly report'")]),
)]
impl Command for DeleteSavedReport {
    type Output = ();

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service
            .delete_saved_report(ctx.org_id(), self.report_id)
            .await
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct RunSavedReport {
    /// Saved report's prefixed public identifier.
    pub report_id: Uuid,
}

#[command(
    name = "run_saved_report",
    category = "reporting",
    description = "Run an org-scoped saved report definition.",
    method = "POST",
    path = "/v1/reports/saved/{report_id}/run",
    policy = REPORT_VIEW,
    cli = CliRoute::new(&["reports", "saved"], "run").with_examples(&[CliExample::new("Get the current numbers for a saved report", "everruns reports saved run --report-id 0190f8a2-7c1e-7d3a-9b2f-4e5d6c7b8a90")]),
    read_only = true,
)]
impl Command for RunSavedReport {
    type Output = ReportResult;

    async fn execute(self, ctx: &Ctx) -> Result<ReportResult, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service
            .run_saved_report(
                ReportScope {
                    org_id: ctx.org_id(),
                    caller: ctx.caller.clone(),
                },
                self.report_id,
            )
            .await
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ExportReportQuery(pub ExportReportQueryRequest);

#[command(
    name = "export_report_query",
    category = "reporting",
    description = "Run and export an org-scoped semantic reporting query.",
    method = "POST",
    path = "/v1/reports/query/export",
    policy = REPORT_VIEW,
    cli = CliRoute::new(&["reports", "query"], "export").with_examples(&[CliExample::new("Download a one-off query result as CSV", "everruns reports query export --format csv --query '{\"dataset\":\"sessions\",\"time_range\":{\"from\":\"2026-04-01T00:00:00Z\",\"to\":\"2026-05-01T00:00:00Z\"},\"dimensions\":[\"status\"],\"measures\":[\"session_count\"],\"filters\":[],\"order_by\":[],\"limit\":100}'")]),
    read_only = true,
)]
impl Command for ExportReportQuery {
    type Output = ReportExport;

    async fn execute(self, ctx: &Ctx) -> Result<ReportExport, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service
            .export_query(
                ReportScope {
                    org_id: ctx.org_id(),
                    caller: ctx.caller.clone(),
                },
                self.0.query,
                self.0.format,
                "report",
            )
            .await
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ExportSavedReport {
    /// Saved report's prefixed public identifier.
    pub report_id: Uuid,
    pub request: ExportSavedReportRequest,
}

#[command(
    name = "export_saved_report",
    category = "reporting",
    description = "Run and export an org-scoped saved report definition.",
    method = "POST",
    path = "/v1/reports/saved/{report_id}/export",
    policy = REPORT_VIEW,
    cli = CliRoute::new(&["reports", "saved"], "export").with_examples(&[CliExample::new("Download a saved report's current data as JSON", "everruns reports saved export --report-id 0190f8a2-7c1e-7d3a-9b2f-4e5d6c7b8a90 --request '{\"format\":\"json\"}'")]),
    read_only = true,
)]
impl Command for ExportSavedReport {
    type Output = ReportExport;

    async fn execute(self, ctx: &Ctx) -> Result<ReportExport, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service
            .export_saved_report(
                ReportScope {
                    org_id: ctx.org_id(),
                    caller: ctx.caller.clone(),
                },
                self.report_id,
                self.request.format,
            )
            .await
    }
}

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct GetReportingDiagnostics;

#[command(
    name = "get_reporting_diagnostics",
    category = "reporting",
    description = "Inspect reporting projector lag and failed outbox rows.",
    method = "GET",
    path = "/v1/reports/admin/diagnostics",
    policy = REPORT_ADMIN,
    cli = CliRoute::new(&["reports", "admin", "diagnostics"], "get").with_examples(&[CliExample::new("Check whether reporting is lagging behind live data", "everruns reports admin diagnostics get")]),
)]
impl Command for GetReportingDiagnostics {
    type Output = ReportingDiagnostics;

    async fn execute(self, ctx: &Ctx) -> Result<ReportingDiagnostics, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service.diagnostics(ctx.org_id()).await
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct RunReportingProjector {
    #[serde(default = "default_projector_limit")]
    /// Maximum number of items returned in this page.
    pub limit: i64,
}

fn default_projector_limit() -> i64 {
    100
}

#[command(
    name = "run_reporting_projector",
    category = "reporting",
    description = "Claim and process pending reporting outbox rows.",
    method = "POST",
    path = "/v1/reports/projector/run",
    policy = REPORT_ADMIN,
    cli = CliRoute::new(&["reports", "projector"], "run").with_examples(&[CliExample::new("Process pending reporting rows right away when reports look stale", "everruns reports projector run --limit 500 --reason 'Reports lag behind live data'")]),
)]
impl Command for RunReportingProjector {
    type Output = ProjectorRunResult;

    async fn execute(self, ctx: &Ctx) -> Result<ProjectorRunResult, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service.run_projector_once(ctx.org_id(), self.limit).await
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct BackfillReporting(pub ReportingBackfillRequest);

#[command(
    name = "backfill_reporting",
    category = "reporting",
    description = "Enqueue missing reporting projection work from canonical sources.",
    method = "POST",
    path = "/v1/reports/admin/backfill",
    policy = REPORT_ADMIN,
    cli = CliRoute::new(&["reports", "admin"], "backfill").with_examples(&[CliExample::new("Rebuild reporting data that is missing from older sessions", "everruns reports admin backfill --limit 1000 --reason 'Reports are missing last week'")]),
)]
impl Command for BackfillReporting {
    type Output = ReportingBackfillResult;

    async fn execute(self, ctx: &Ctx) -> Result<ReportingBackfillResult, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service
            .backfill_missing(Some(ctx.org_id()), self.0.limit)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_report_reads_are_available_to_mcp_query() {
        assert!(RunReportQuery::read_only());
        assert!(RunSavedReport::read_only());
        assert!(ExportReportQuery::read_only());
        assert!(ExportSavedReport::read_only());
    }
}
