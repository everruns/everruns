// Sandbox Template and primary Sandbox domain commands.

use serde::Deserialize;
use utoipa::ToSchema;

use super::queries::effective_session_capabilities;
use super::resolve::{sandbox_from_capabilities, sandbox_from_record, sandbox_targets};
use crate::domains::common::*;
use crate::domains::sandbox_templates::types::{SandboxTargetsResponse, SessionSandboxResponse};

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetSessionSandbox {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "get_session_sandbox",
    category = "sandboxes",
    description = "Inspect a Session's primary Sandbox and what it may touch.",
    method = "GET",
    path = "/v1/sessions/{session_id}/sandbox",
    policy = crate::domains::sessions::SESSION_VIEW,
    positional = "session_id",
)]
impl Command for GetSessionSandbox {
    type Output = SessionSandboxResponse;

    async fn execute(self, ctx: &Ctx) -> Result<SessionSandboxResponse, CommandError> {
        let session_id = crate::domains::sessions::queries::parse_session_id(&self.session_id)?;
        let _ = crate::domains::sessions::queries::get_session(ctx, session_id, ctx.caller.user_id)
            .await?;

        let capabilities = effective_session_capabilities(&ctx.db, session_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session not found"))?;

        let sandbox = ctx.db.get_primary_sandbox(session_id).await?;
        Ok(match sandbox {
            Some(record) => sandbox_from_record(&record, &capabilities),
            None => sandbox_from_capabilities(&capabilities),
        })
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListSandboxTargets;

#[command(
    name = "list_sandbox_targets",
    category = "sandbox-templates",
    description = "List the Sandbox targets this deployment can offer.",
    method = "GET",
    path = "/v1/sandbox-targets"
)]
impl Command for ListSandboxTargets {
    type Output = SandboxTargetsResponse;

    async fn execute(self, _ctx: &Ctx) -> Result<SandboxTargetsResponse, CommandError> {
        Ok(SandboxTargetsResponse {
            items: sandbox_targets(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_commands_are_part_of_the_core_surface() {
        assert_eq!(GetSessionSandbox::meta().required_feature(), None);
        assert_eq!(ListSandboxTargets::meta().required_feature(), None);
    }
}
