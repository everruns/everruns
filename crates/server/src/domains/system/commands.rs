use super::doctor::DoctorReport;
use super::queries as q;
use super::types::HealthCheckResponse;
use crate::domains::common::*;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct HealthCheck {}

#[command(
    name = "health_check",
    category = "system",
    description = "Health check endpoint.",
    method = "GET",
    path = "/health",
    cli = CliRoute::new(&["system"], "health").with_examples(&[CliExample::new("Check that the control plane is reachable and healthy", "everruns system health",)]),
)]
impl Command for HealthCheck {
    type Output = HealthCheckResponse;

    async fn execute(self, _ctx: &Ctx) -> Result<HealthCheckResponse, CommandError> {
        Ok(HealthCheckResponse {
            status: "ok",
            version: q::version(),
        })
    }
}

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct RunSystemDoctor {
    /// Run every check now instead of returning the report from the last
    /// minute. Refreshes closer than five seconds apart share one run.
    #[serde(default)]
    pub refresh: Option<bool>,
}

#[command(
    name = "run_system_doctor",
    category = "system",
    description = "Check whether the whole install is set up right and working: database, workers, keys, messaging, storage, monitoring.",
    method = "GET",
    path = "/v1/system/doctor",
    policy = super::SYSTEM_VIEW,
    cli = CliRoute::new(&["system"], "doctor").with_examples(&[CliExample::new("Check every part of the install and list what needs fixing", "everruns system doctor --refresh true",)]),
)]
impl Command for RunSystemDoctor {
    type Output = DoctorReport;

    async fn execute(self, ctx: &Ctx) -> Result<DoctorReport, CommandError> {
        let doctor = ctx.system_doctor.as_ref().ok_or_else(|| {
            CommandError::unavailable("The system doctor is not available on this surface")
        })?;
        Ok(doctor.report(self.refresh.unwrap_or(false)).await)
    }
}

/// Page through the open findings that concern the platform operator.
#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct ListPlatformDetections {
    /// Number of detections to skip; defaults to zero.
    #[serde(default)]
    pub offset: Option<i64>,
    /// Page size from 1 through 100; defaults to 20.
    #[serde(default)]
    pub limit: Option<i64>,
}

#[command(
    name = "list_platform_detections",
    category = "system",
    description = "List open abuse detections in every organization, with the Inspector's write-up. Leads for a person to confirm, never grounds for automatic action.",
    method = "GET",
    path = "/v1/system/detections",
    policy = super::SYSTEM_VIEW,
    cli = CliRoute::new(&["system"], "detections").with_examples(&[CliExample::new("Review sessions suspected of abusing the platform", "everruns system detections --limit 20",)]),
)]
impl Command for ListPlatformDetections {
    type Output = crate::domains::health_issues::types::PlatformDetectionList;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        use crate::domains::health_issues::inspector::PLATFORM_CODES;
        use crate::domains::health_issues::types::{PlatformDetection, PlatformDetectionList};
        let offset = self.offset.unwrap_or(0).max(0);
        let limit = self.limit.unwrap_or(20).clamp(1, 100);
        let rows = ctx
            .db
            .platform_detections(&PLATFORM_CODES, offset, limit)
            .await?;
        let total = ctx.db.count_platform_detections(&PLATFORM_CODES).await?;
        Ok(PlatformDetectionList {
            data: rows
                .into_iter()
                .map(|r| PlatformDetection {
                    issue_id: r.health_issue_id,
                    org_id: r.org_public_id,
                    org_name: r.org_name,
                    code: r.code,
                    session_id: r.session_id.map(|id| {
                        everruns_contracts::typed_id::SessionId::from_uuid(id).to_string()
                    }),
                    agent_name: r.agent_name,
                    evidence: r.evidence,
                    first_detected_at: r.first_detected_at,
                    what_happened: r.what_happened,
                    severity: r.severity,
                    suggested_action: r.suggested_action,
                    cluster_title: r.cluster_title,
                })
                .collect(),
            total,
            offset,
            limit,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::domains::common::Ctx;
    use crate::storage::StorageBackend;
    use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole};
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn health_check_dispatch_accepts_empty_object_params() {
        let db = Arc::new(StorageBackend::test_database());
        let ctx = Ctx::minimal_for_test(
            Caller {
                org_id: DEFAULT_ORG_ID,
                org_public_id: "org_00000000000000000000000000000001".to_string(),
                user_id: None,
                role: OrgRole::Owner,
                is_platform_user: false,
                is_internal: false,
                read_only: false,
            },
            db,
            None,
        );

        let json = crate::domains::common::dispatch("health_check", json!({}), &ctx)
            .await
            .expect("dispatch health_check");
        let response: serde_json::Value =
            serde_json::from_str(&json).expect("deserialize health_check response");
        assert_eq!(response.get("status"), Some(&json!("ok")));
    }
}
