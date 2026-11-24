use crate::domains::common::*;
use crate::domains::observers::{
    record::TraceScore,
    service::{OBSERVER_VIEW, row_to_trace_score},
};
use crate::storage::{
    ListTraceScoresParams,
    observer_monitoring::{MonitoringCheckTotals, MonitoringSpend},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Organization consent, monitoring coverage and current daily spending.
#[derive(Debug, Serialize, ToSchema)]
pub struct QualityMonitoringReport {
    /// Current organization opt-in and assessment settings.
    pub policy: crate::domains::observers::system::QualityMonitoringPolicy,
    /// Managed observer identity, absent until monitoring has been configured.
    #[schema(example = "observer_01933b5a000070008000000000000001")]
    pub observer_id: Option<String>,
    /// Conservative admission reservations and provider-reported cost for this UTC day.
    pub today: MonitoringSpend,
    /// Separate execution and continuity verdict counts over seven days.
    pub checks: Vec<MonitoringCheckTotals>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetOrgQualityMonitoring {
    /// Organization whose monitoring policy and coverage to inspect.
    #[schema(example = "org_00000000000000000000000000000001")]
    pub org: String,
}

#[command(name = "get_org_quality_monitoring", summary = "Get compaction monitoring", cli = CliRoute::new(&["orgs", "quality-monitoring"], "get").with_examples(&[CliExample::new("Inspect compaction monitoring consent, coverage and allowance", "everruns orgs quality-monitoring get --org org_01h9")]), category = "organizations", description = "Read organization compaction monitoring policy, coverage and budget reservations.", method = "GET", path = "/v1/platform/orgs/{org}/quality-monitoring", policy = super::settings::ORGANIZATION_VIEW, http = plain, tag = "Organizations")]
impl Command for GetOrgQualityMonitoring {
    type Output = QualityMonitoringReport;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if self.org != ctx.caller.org_public_id {
            return Err(CommandError::not_found("Organization"));
        }
        Ok(QualityMonitoringReport {
            policy: crate::domains::observers::system::policy(&ctx.db, ctx.org_id()).await?,
            observer_id: ctx
                .db
                .system_compaction_observer(ctx.org_id())
                .await?
                .map(|row| row.public_id),
            today: ctx.db.monitoring_spend(ctx.org_id()).await?,
            checks: ctx.db.compaction_monitoring_totals(ctx.org_id()).await?,
        })
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListOrgQualityMonitoringScores {
    /// Organization whose newest monitoring findings to inspect.
    #[schema(example = "org_00000000000000000000000000000001")]
    pub org: String,
}

#[command(name = "list_org_quality_monitoring_scores", summary = "List compaction findings", cli = CliRoute::new(&["orgs", "quality-monitoring", "scores"], "list").with_examples(&[CliExample::new("Read recent compaction assessments and their trace references", "everruns orgs quality-monitoring scores list --org org_01h9")]), category = "organizations", description = "Read the newest fifty compaction monitoring findings with source trace references.", method = "GET", path = "/v1/platform/orgs/{org}/quality-monitoring/scores", policy = OBSERVER_VIEW, http = plain, tag = "Organizations")]
impl Command for ListOrgQualityMonitoringScores {
    type Output = Vec<TraceScore>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if self.org != ctx.caller.org_public_id {
            return Err(CommandError::not_found("Organization"));
        }
        let Some(observer) = ctx.db.system_compaction_observer(ctx.org_id()).await? else {
            return Ok(Vec::new());
        };
        let public_id = observer.public_id.parse().map_err(|_| {
            CommandError::internal(anyhow::anyhow!("Invalid managed observer identity"))
        })?;
        ctx.db
            .list_trace_scores(
                ctx.org_id(),
                ListTraceScoresParams {
                    observer_id: observer.id,
                    limit: 50,
                    ..Default::default()
                },
            )
            .await?
            .into_iter()
            .map(|row| row_to_trace_score(row, public_id).map_err(classify_anyhow))
            .collect()
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReviewOrgQualityMonitoringScore {
    /// Organization that owns the assessment.
    #[schema(example = "org_00000000000000000000000000000001")]
    pub org: String,
    /// Assessment whose human review to record.
    #[schema(example = "score_01933b5a000070008000000000000001")]
    pub score: String,
    #[serde(flatten)]
    pub review: crate::domains::observers::types::ReviewScoreRequest,
}

#[command(name = "review_org_quality_monitoring_score", summary = "Review a compaction finding", cli = CliRoute::new(&["orgs", "quality-monitoring", "scores"], "review").with_examples(&[CliExample::new("Record a suspected continuity issue as a grader mistake", "everruns orgs quality-monitoring scores review --org org_01h9 --score score_01h9 --agrees false --note 'A later user correction explains the answer' --reason 'Calibrate compaction monitoring'")]), category = "organizations", description = "Record a person's agreement with a compaction assessment, without starting model work.", method = "PUT", path = "/v1/platform/orgs/{org}/quality-monitoring/scores/{score}/review", policy = OBSERVER_VIEW, http = plain, tag = "Organizations")]
impl Command for ReviewOrgQualityMonitoringScore {
    type Output = Option<TraceScore>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if self.org != ctx.caller.org_public_id {
            return Err(CommandError::not_found("Organization"));
        }
        let Some(observer) = ctx.db.system_compaction_observer(ctx.org_id()).await? else {
            return Ok(None);
        };
        crate::domains::observers::ObserverService::new(ctx.db.clone())
            .review_score(&ctx.caller, &observer.public_id, &self.score, self.review)
            .await
            .map_err(classify_anyhow)
    }
}
