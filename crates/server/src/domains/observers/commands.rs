use super::service::{OBSERVER_MANAGE, OBSERVER_VIEW, ObserverService};
use super::types::{
    BackfillObserverRequest, CreateObserverRequest, ListObserversQuery, QualitySummaryQuery,
    ReviewScoreRequest, UpdateObserverRequest,
};
use crate::domains::common::*;
use crate::domains::observers::record::{BackfillResult, Observer, QualitySummary, TraceScore};
use everruns_contracts::typed_id::{AgentId, ObserverId, SessionId};
use serde::Deserialize;
use utoipa::ToSchema;

fn service(ctx: &Ctx) -> ObserverService {
    ObserverService::new(ctx.db.clone())
}

fn parse_observer_id(id: &str) -> Result<ObserverId, CommandError> {
    id.parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid observer ID: {e}")))
}

// ============================================
// Create
// ============================================

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct CreateObserver(pub CreateObserverRequest);

impl CommandSchema for CreateObserver {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateObserverRequest>()
    }
}

#[command(
    name = "create_observer",
    category = "observers",
    description = "Create an observer (online scoring).",
    method = "POST",
    path = "/v1/observers",
    policy = OBSERVER_MANAGE,
    cli = CliRoute::new(&["observers"], "create").with_examples(&[CliExample::new("Score a sample of production turns automatically", "everruns observers create --name helpfulness --sampling-rate 0.1 --scorers '[{\"key\":\"mentions_docs\",\"method\":\"rule\",\"rule\":{\"type\":\"contains\",\"text\":\"docs\"}}]' --reason 'Track answer quality'")]),
    http = created,
    request_body(CreateObserverRequest),
)]
impl Command for CreateObserver {
    type Output = Observer;

    async fn execute(self, ctx: &Ctx) -> Result<Observer, CommandError> {
        service(ctx)
            .create(&ctx.caller, self.0)
            .await
            .map_err(classify_anyhow)
    }
}

// ============================================
// List
// ============================================

#[derive(Debug, Default, Deserialize, serde::Serialize)]
pub struct ListObservers {
    /// Also return archived items.
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
}

impl CommandSchema for ListObservers {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<ListObserversQuery>()
    }
}

#[command(
    name = "list_observers",
    category = "observers",
    description = "List observers.",
    method = "GET",
    path = "/v1/observers",
    policy = OBSERVER_VIEW,
    cli = CliRoute::new(&["observers"], "list").with_examples(&[CliExample::new("See which observers are scoring production sessions", "everruns observers list")]),
    http = list,
    params(ListObserversQuery),
)]
impl Command for ListObservers {
    type Output = Vec<Observer>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Observer>, CommandError> {
        service(ctx)
            .list(&ctx.caller, self.include_archived)
            .await
            .map_err(classify_anyhow)
    }
}

// ============================================
// Get
// ============================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetObserver {
    /// Observer's prefixed public identifier.
    pub observer_id: String,
}

#[command(
    name = "get_observer",
    category = "observers",
    description = "Get a single observer.",
    method = "GET",
    path = "/v1/observers/{observer_id}",
    policy = OBSERVER_VIEW,
    cli = CliRoute::new(&["observers"], "get").with_args(&[CliArg::new("observer_id").at(1)]).with_examples(&[CliExample::new("Check an observer's scorers, match rules and sampling rate", "everruns observers get observer_01h9")]),
    positional = "observer_id",
    http = plain,
    responses((status = 404, description = "Observer not found")),
)]
impl Command for GetObserver {
    type Output = Observer;

    async fn execute(self, ctx: &Ctx) -> Result<Observer, CommandError> {
        let observer_id = parse_observer_id(&self.observer_id)?;
        service(ctx)
            .get_by_public_id(&ctx.caller, &observer_id.to_string())
            .await?
            .ok_or_else(|| CommandError::not_found("Observer"))
    }
}

// ============================================
// Update
// ============================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateObserver {
    /// Observer's prefixed public identifier.
    pub observer_id: String,
    #[serde(flatten)]
    pub req: UpdateObserverRequest,
}

#[command(
    name = "update_observer",
    category = "observers",
    description = "Update an observer.",
    method = "PATCH",
    path = "/v1/observers/{observer_id}",
    policy = OBSERVER_MANAGE,
    cli = CliRoute::new(&["observers"], "update").with_examples(&[CliExample::new("Score fewer turns to cut judge cost", "everruns observers update --observer-id observer_01h9 --sampling-rate 0.05 --reason 'Reduce scoring volume'")]),
    http = plain,
    request_body(UpdateObserverRequest),
    responses((status = 404, description = "Observer not found")),
)]
impl Command for UpdateObserver {
    type Output = Observer;

    async fn execute(self, ctx: &Ctx) -> Result<Observer, CommandError> {
        let observer_id = parse_observer_id(&self.observer_id)?;
        service(ctx)
            .update(&ctx.caller, &observer_id.to_string(), self.req)
            .await?
            .ok_or_else(|| CommandError::not_found("Observer"))
    }
}

// ============================================
// Delete
// ============================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteObserver {
    /// Observer's prefixed public identifier.
    pub observer_id: String,
}

#[command(
    name = "delete_observer",
    category = "observers",
    description = "Archive an observer.",
    method = "DELETE",
    path = "/v1/observers/{observer_id}",
    policy = OBSERVER_MANAGE,
    cli = CliRoute::new(&["observers"], "delete").with_examples(&[CliExample::new("Archive an observer you no longer want scoring turns", "everruns observers delete --observer-id observer_01h9 --reason 'Replaced by a stricter observer'")]),
    http = no_content,
    responses((status = 404, description = "Observer not found")),
)]
impl Command for DeleteObserver {
    type Output = bool;

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let observer_id = parse_observer_id(&self.observer_id)?;
        let deleted = service(ctx)
            .delete(&ctx.caller, &observer_id.to_string())
            .await?;
        if deleted {
            Ok(true)
        } else {
            Err(CommandError::not_found("Observer"))
        }
    }
}

// ============================================
// List scores
// ============================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListObserverScores {
    /// Observer's prefixed public identifier.
    pub observer_id: String,
    /// Session's prefixed public identifier.
    pub session_id: Option<String>,
    #[serde(default = "default_scores_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    /// Only unreviewed completed scores, failures first (the review queue).
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub review_queue: bool,
}

const fn default_scores_limit() -> i64 {
    100
}

#[command(
    name = "list_observer_scores",
    category = "observers",
    description = "List trace scores produced by an observer.",
    method = "GET",
    path = "/v1/observers/{observer_id}/scores",
    policy = OBSERVER_VIEW,
    cli = CliRoute::new(&["observers", "scores"], "list").with_examples(&[CliExample::new("Look at the scores an observer gave one session", "everruns observers scores list --observer-id observer_01h9 --session-id session_01h9")]),
    http = list,
    params(crate::domains::observers::types::ListTraceScoresQuery),
)]
impl Command for ListObserverScores {
    type Output = Vec<TraceScore>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<TraceScore>, CommandError> {
        let observer_id = parse_observer_id(&self.observer_id)?;
        let session_id = self
            .session_id
            .as_deref()
            .map(|s| {
                s.parse::<SessionId>()
                    .map_err(|e| CommandError::bad_request(format!("Invalid session ID: {e}")))
            })
            .transpose()?;
        service(ctx)
            .list_scores(
                &ctx.caller,
                &observer_id.to_string(),
                session_id,
                self.review_queue,
                self.limit,
                self.offset,
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Observer"))
    }
}

// ============================================
// Review a score
// ============================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ReviewObserverScore {
    /// Observer's prefixed public identifier.
    pub observer_id: String,
    /// Score's prefixed public identifier.
    pub score_id: String,
    #[serde(flatten)]
    pub req: ReviewScoreRequest,
}

#[command(
    name = "review_observer_score",
    category = "observers",
    description = "Record whether an observer's score was right.",
    method = "POST",
    path = "/v1/observers/{observer_id}/scores/{score_id}/review",
    policy = OBSERVER_MANAGE,
    cli = CliRoute::new(&["observers", "scores"], "review").with_examples(&[CliExample::new("Mark a failing score as a grader mistake", "everruns observers scores review --observer-id observer_01h9 --score-id score_01h9 --agrees false --note 'The answer cited the refund policy' --reason 'Calibrate the grounded judge'")]),
    http = plain,
    request_body(ReviewScoreRequest),
    responses((status = 404, description = "Observer or completed score not found")),
)]
impl Command for ReviewObserverScore {
    type Output = TraceScore;

    async fn execute(self, ctx: &Ctx) -> Result<TraceScore, CommandError> {
        let observer_id = parse_observer_id(&self.observer_id)?;
        service(ctx)
            .review_score(
                &ctx.caller,
                &observer_id.to_string(),
                &self.score_id,
                self.req,
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Score"))
    }
}

// ============================================
// Backfill
// ============================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct BackfillObserver {
    /// Observer's prefixed public identifier.
    pub observer_id: String,
    #[serde(flatten)]
    pub req: BackfillObserverRequest,
}

#[command(
    name = "backfill_observer",
    category = "observers",
    description = "Score past production traffic with an observer.",
    method = "POST",
    path = "/v1/observers/{observer_id}/backfill",
    policy = OBSERVER_MANAGE,
    cli = CliRoute::new(&["observers"], "backfill").with_examples(&[CliExample::new("Score the last day of traffic with a new observer", "everruns observers backfill --observer-id observer_01h9 --since-hours 24 --reason 'Baseline the new rubric'")]),
    http = plain,
    request_body(BackfillObserverRequest),
    responses((status = 404, description = "Observer not found")),
)]
impl Command for BackfillObserver {
    type Output = BackfillResult;

    async fn execute(self, ctx: &Ctx) -> Result<BackfillResult, CommandError> {
        let observer_id = parse_observer_id(&self.observer_id)?;
        service(ctx)
            .backfill(&ctx.caller, &observer_id.to_string(), self.req)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Observer"))
    }
}

// ============================================
// Live quality summary
// ============================================

#[derive(Debug, Default, Deserialize, serde::Serialize)]
pub struct GetObserverSummary {
    /// Window in days (1–90). Defaults to 7.
    pub days: Option<u32>,
    /// Only scores of this agent.
    pub agent_id: Option<String>,
    /// Only scores of this observer.
    pub observer_id: Option<String>,
}

impl CommandSchema for GetObserverSummary {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<QualitySummaryQuery>()
    }
}

#[command(
    name = "get_observer_summary",
    category = "observers",
    description = "Live quality by agent, observer and day.",
    method = "GET",
    path = "/v1/observers/summary",
    policy = OBSERVER_VIEW,
    cli = CliRoute::new(&["observers"], "summary").with_examples(&[CliExample::new("See how one agent's production answers scored this week", "everruns observers summary --agent-id agent_01h9 --days 7")]),
    http = plain,
    params(QualitySummaryQuery),
    responses((status = 404, description = "Observer not found")),
)]
impl Command for GetObserverSummary {
    type Output = QualitySummary;

    async fn execute(self, ctx: &Ctx) -> Result<QualitySummary, CommandError> {
        let agent_id = self
            .agent_id
            .as_deref()
            .map(|id| {
                id.parse::<AgentId>()
                    .map(|a| a.uuid())
                    .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))
            })
            .transpose()?;
        let observer_id = self
            .observer_id
            .as_deref()
            .map(parse_observer_id)
            .transpose()?
            .map(|id| id.to_string());
        service(ctx)
            .summary(&ctx.caller, self.days, agent_id, observer_id.as_deref())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Observer"))
    }
}
