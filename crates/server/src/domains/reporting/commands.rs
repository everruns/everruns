use everruns_core::Policy;
use everruns_core::reporting::{DatasetCatalog, ReportQuery, ReportResult, ReportScope};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::catalog;
use super::types::{
    CreateSavedReportRequest, ExportReportQueryRequest, ExportSavedReportRequest,
    ProjectorRunResult, ReportExport, ReportingDiagnostics, SavedReport, UpdateSavedReportRequest,
};
use super::{REPORT_ADMIN, REPORT_MANAGE, REPORT_VIEW};
use crate::domains::common::{Command, CommandDescriptor, CommandError, CommandMeta, Ctx};

#[derive(Debug, Deserialize, ToSchema)]
pub struct RunReportQuery(pub ReportQuery);

impl Command for RunReportQuery {
    type Output = ReportResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "run_report_query",
            category: "reporting",
            description: "Run an org-scoped semantic reporting query.",
            method: "POST",
            path: "/v1/reports/query",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&REPORT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<ReportResult, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::Internal(anyhow::anyhow!("Reporting service not configured"))
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

inventory::submit! { CommandDescriptor::of::<RunReportQuery>() }

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct GetReportCatalog;

impl Command for GetReportCatalog {
    type Output = DatasetCatalog;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_report_catalog",
            category: "reporting",
            description: "Return semantic reporting datasets, dimensions, measures, and filter fields.",
            method: "GET",
            path: "/v1/reports/catalog",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&REPORT_VIEW)
    }

    async fn execute(self, _ctx: &Ctx) -> Result<DatasetCatalog, CommandError> {
        Ok(catalog::catalog())
    }
}

inventory::submit! { CommandDescriptor::of::<GetReportCatalog>() }

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListSavedReports;

impl Command for ListSavedReports {
    type Output = Vec<SavedReport>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_saved_reports",
            category: "reporting",
            description: "List org-scoped saved report definitions.",
            method: "GET",
            path: "/v1/reports/saved",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&REPORT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SavedReport>, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::Internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service.list_saved_reports(ctx.org_id()).await
    }
}

inventory::submit! { CommandDescriptor::of::<ListSavedReports>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetSavedReport {
    pub report_id: Uuid,
}

impl Command for GetSavedReport {
    type Output = SavedReport;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_saved_report",
            category: "reporting",
            description: "Get an org-scoped saved report definition.",
            method: "GET",
            path: "/v1/reports/saved/{report_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&REPORT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SavedReport, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::Internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service.get_saved_report(ctx.org_id(), self.report_id).await
    }
}

inventory::submit! { CommandDescriptor::of::<GetSavedReport>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSavedReport(pub CreateSavedReportRequest);

impl Command for CreateSavedReport {
    type Output = SavedReport;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_saved_report",
            category: "reporting",
            description: "Create an org-scoped saved report definition.",
            method: "POST",
            path: "/v1/reports/saved",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&REPORT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SavedReport, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::Internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service.create_saved_report(ctx.org_id(), self.0).await
    }
}

inventory::submit! { CommandDescriptor::of::<CreateSavedReport>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSavedReport {
    pub report_id: Uuid,
    pub request: UpdateSavedReportRequest,
}

impl Command for UpdateSavedReport {
    type Output = SavedReport;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_saved_report",
            category: "reporting",
            description: "Update an org-scoped saved report definition.",
            method: "PATCH",
            path: "/v1/reports/saved/{report_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&REPORT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SavedReport, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::Internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service
            .update_saved_report(ctx.org_id(), self.report_id, self.request)
            .await
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateSavedReport>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteSavedReport {
    pub report_id: Uuid,
}

impl Command for DeleteSavedReport {
    type Output = ();

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_saved_report",
            category: "reporting",
            description: "Delete an org-scoped saved report definition.",
            method: "DELETE",
            path: "/v1/reports/saved/{report_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&REPORT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::Internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service
            .delete_saved_report(ctx.org_id(), self.report_id)
            .await
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteSavedReport>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct RunSavedReport {
    pub report_id: Uuid,
}

impl Command for RunSavedReport {
    type Output = ReportResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "run_saved_report",
            category: "reporting",
            description: "Run an org-scoped saved report definition.",
            method: "POST",
            path: "/v1/reports/saved/{report_id}/run",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&REPORT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<ReportResult, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::Internal(anyhow::anyhow!("Reporting service not configured"))
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

inventory::submit! { CommandDescriptor::of::<RunSavedReport>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExportReportQuery(pub ExportReportQueryRequest);

impl Command for ExportReportQuery {
    type Output = ReportExport;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "export_report_query",
            category: "reporting",
            description: "Run and export an org-scoped semantic reporting query.",
            method: "POST",
            path: "/v1/reports/query/export",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&REPORT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<ReportExport, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::Internal(anyhow::anyhow!("Reporting service not configured"))
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

inventory::submit! { CommandDescriptor::of::<ExportReportQuery>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExportSavedReport {
    pub report_id: Uuid,
    pub request: ExportSavedReportRequest,
}

impl Command for ExportSavedReport {
    type Output = ReportExport;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "export_saved_report",
            category: "reporting",
            description: "Run and export an org-scoped saved report definition.",
            method: "POST",
            path: "/v1/reports/saved/{report_id}/export",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&REPORT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<ReportExport, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::Internal(anyhow::anyhow!("Reporting service not configured"))
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

inventory::submit! { CommandDescriptor::of::<ExportSavedReport>() }

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct GetReportingDiagnostics;

impl Command for GetReportingDiagnostics {
    type Output = ReportingDiagnostics;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_reporting_diagnostics",
            category: "reporting",
            description: "Inspect reporting projector lag and failed outbox rows.",
            method: "GET",
            path: "/v1/reports/admin/diagnostics",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&REPORT_ADMIN)
    }

    async fn execute(self, ctx: &Ctx) -> Result<ReportingDiagnostics, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::Internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service.diagnostics(ctx.org_id()).await
    }
}

inventory::submit! { CommandDescriptor::of::<GetReportingDiagnostics>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct RunReportingProjector {
    #[serde(default = "default_projector_limit")]
    pub limit: i64,
}

fn default_projector_limit() -> i64 {
    100
}

impl Command for RunReportingProjector {
    type Output = ProjectorRunResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "run_reporting_projector",
            category: "reporting",
            description: "Claim and process pending reporting outbox rows.",
            method: "POST",
            path: "/v1/reports/projector/run",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&REPORT_ADMIN)
    }

    async fn execute(self, ctx: &Ctx) -> Result<ProjectorRunResult, CommandError> {
        let service = ctx.reporting_service.as_ref().ok_or_else(|| {
            CommandError::Internal(anyhow::anyhow!("Reporting service not configured"))
        })?;
        service.run_projector_once(ctx.org_id(), self.limit).await
    }
}

inventory::submit! { CommandDescriptor::of::<RunReportingProjector>() }
