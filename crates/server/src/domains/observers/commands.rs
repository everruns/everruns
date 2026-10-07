use super::service::{OBSERVER_MANAGE, OBSERVER_VIEW, ObserverService};
use super::types::{CreateObserverRequest, ListObserversQuery, UpdateObserverRequest};
use crate::domains::common::*;
use crate::records::observer::{Observer, TraceScore};
use everruns_contracts::typed_id::{ObserverId, SessionId};
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
    pub observer_id: String,
}

#[command(
    name = "get_observer",
    category = "observers",
    description = "Get a single observer.",
    method = "GET",
    path = "/v1/observers/{observer_id}",
    policy = OBSERVER_VIEW,
    positional = "observer_id",
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
    pub observer_id: String,
}

#[command(
    name = "delete_observer",
    category = "observers",
    description = "Archive an observer.",
    method = "DELETE",
    path = "/v1/observers/{observer_id}",
    policy = OBSERVER_MANAGE,
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
    pub observer_id: String,
    pub session_id: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

#[command(
    name = "list_observer_scores",
    category = "observers",
    description = "List trace scores produced by an observer.",
    method = "GET",
    path = "/v1/observers/{observer_id}/scores",
    policy = OBSERVER_VIEW,
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
                self.limit,
                self.offset,
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Observer"))
    }
}
