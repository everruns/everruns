use super::service::{OBSERVER_MANAGE, OBSERVER_VIEW, ObserverService};
use super::types::{CreateObserverRequest, ListObserversQuery, UpdateObserverRequest};
use crate::domains::common::*;
use everruns_core::typed_id::{ObserverId, SessionId};
use everruns_platform::observer::{Observer, TraceScore};
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

#[derive(Debug, Deserialize)]
pub struct CreateObserver(pub CreateObserverRequest);

impl CommandSchema for CreateObserver {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateObserverRequest>()
    }
}

impl Command for CreateObserver {
    type Output = Observer;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_observer",
            category: "observers",
            description: "Create an observer (online scoring).",
            method: "POST",
            path: "/v1/observers",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&OBSERVER_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Observer, CommandError> {
        service(ctx)
            .create(&ctx.caller, self.0)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateObserver>() }

// ============================================
// List
// ============================================

#[derive(Debug, Default, Deserialize)]
pub struct ListObservers {
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
}

impl CommandSchema for ListObservers {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<ListObserversQuery>()
    }
}

impl Command for ListObservers {
    type Output = Vec<Observer>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_observers",
            category: "observers",
            description: "List observers.",
            method: "GET",
            path: "/v1/observers",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&OBSERVER_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Observer>, CommandError> {
        service(ctx)
            .list(&ctx.caller, self.include_archived)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ListObservers>() }

// ============================================
// Get
// ============================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetObserver {
    pub observer_id: String,
}

impl Command for GetObserver {
    type Output = Observer;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_observer",
            category: "observers",
            description: "Get a single observer.",
            method: "GET",
            path: "/v1/observers/{observer_id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("observer_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&OBSERVER_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Observer, CommandError> {
        let observer_id = parse_observer_id(&self.observer_id)?;
        service(ctx)
            .get_by_public_id(&ctx.caller, &observer_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Observer"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetObserver>() }

// ============================================
// Update
// ============================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateObserver {
    pub observer_id: String,
    #[serde(flatten)]
    pub req: UpdateObserverRequest,
}

impl Command for UpdateObserver {
    type Output = Observer;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_observer",
            category: "observers",
            description: "Update an observer.",
            method: "PATCH",
            path: "/v1/observers/{observer_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&OBSERVER_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Observer, CommandError> {
        let observer_id = parse_observer_id(&self.observer_id)?;
        service(ctx)
            .update(&ctx.caller, &observer_id.to_string(), self.req)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Observer"))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateObserver>() }

// ============================================
// Delete
// ============================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteObserver {
    pub observer_id: String,
}

impl Command for DeleteObserver {
    type Output = bool;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_observer",
            category: "observers",
            description: "Archive an observer.",
            method: "DELETE",
            path: "/v1/observers/{observer_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&OBSERVER_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let observer_id = parse_observer_id(&self.observer_id)?;
        let deleted = service(ctx)
            .delete(&ctx.caller, &observer_id.to_string())
            .await
            .map_err(classify_anyhow)?;
        if deleted {
            Ok(true)
        } else {
            Err(CommandError::not_found("Observer"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteObserver>() }

// ============================================
// List scores
// ============================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListObserverScores {
    pub observer_id: String,
    pub session_id: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

impl Command for ListObserverScores {
    type Output = Vec<TraceScore>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_observer_scores",
            category: "observers",
            description: "List trace scores produced by an observer.",
            method: "GET",
            path: "/v1/observers/{observer_id}/scores",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&OBSERVER_VIEW)
    }

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
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Observer"))
    }
}

inventory::submit! { CommandDescriptor::of::<ListObserverScores>() }
