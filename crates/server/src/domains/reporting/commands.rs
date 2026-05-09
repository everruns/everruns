use everruns_core::Policy;
use everruns_core::reporting::{DatasetCatalog, ReportQuery, ReportResult, ReportScope};
use serde::Deserialize;
use utoipa::ToSchema;

use super::catalog;
use super::types::ProjectorRunResult;
use super::{REPORT_ADMIN, REPORT_VIEW};
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
        service.run_projector_once(self.limit).await
    }
}

inventory::submit! { CommandDescriptor::of::<RunReportingProjector>() }
