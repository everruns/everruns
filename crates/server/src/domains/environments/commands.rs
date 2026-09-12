// Environment domain commands.

use serde::Deserialize;
use utoipa::ToSchema;

use super::queries::effective_session_capabilities;
use super::resolve::{environment_from_capabilities, environment_targets};
use crate::api::environments::{EnvironmentTargetsResponse, SessionEnvironmentResponse};
use crate::domains::common::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetSessionEnvironment {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for GetSessionEnvironment {
    type Output = SessionEnvironmentResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session_environment",
            category: "environments",
            description: "Inspect where a session's commands run and what they may touch.",
            method: "GET",
            path: "/v1/sessions/{session_id}/environment",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionEnvironmentResponse, CommandError> {
        let session_id = crate::domains::sessions::queries::parse_session_id(&self.session_id)?;
        let _ = crate::domains::sessions::queries::get_session(ctx, session_id, ctx.caller.user_id)
            .await?;

        let capabilities = effective_session_capabilities(&ctx.db, session_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session not found"))?;

        Ok(environment_from_capabilities(&capabilities))
    }
}

inventory::submit! { CommandDescriptor::of::<GetSessionEnvironment>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListEnvironmentTargets;

impl Command for ListEnvironmentTargets {
    type Output = EnvironmentTargetsResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_environment_targets",
            category: "environments",
            description: "List the environment targets this deployment can offer.",
            method: "GET",
            path: "/v1/environment-targets",
        }
    }

    async fn execute(self, _ctx: &Ctx) -> Result<EnvironmentTargetsResponse, CommandError> {
        Ok(EnvironmentTargetsResponse {
            items: environment_targets(),
        })
    }
}

inventory::submit! { CommandDescriptor::of::<ListEnvironmentTargets>() }
