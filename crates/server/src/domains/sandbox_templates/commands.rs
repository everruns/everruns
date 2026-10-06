// Sandbox Template and primary Sandbox domain commands.

use serde::Deserialize;
use utoipa::ToSchema;

use super::queries::effective_session_capabilities;
use super::resolve::{sandbox_from_capabilities, sandbox_from_record, sandbox_targets};
use crate::api::sandbox_templates::{SandboxTargetsResponse, SessionSandboxResponse};
use crate::domains::common::*;

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetSessionSandbox {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for GetSessionSandbox {
    type Output = SessionSandboxResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session_sandbox",
            category: "sandboxes",
            description: "Inspect a Session's primary Sandbox and what it may touch.",
            method: "GET",
            path: "/v1/sessions/{session_id}/sandbox",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

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

inventory::submit! { CommandDescriptor::of::<GetSessionSandbox>() }

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListSandboxTargets;

impl Command for ListSandboxTargets {
    type Output = SandboxTargetsResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_sandbox_targets",
            category: "sandbox-templates",
            description: "List the Sandbox targets this deployment can offer.",
            method: "GET",
            path: "/v1/sandbox-targets",
        }
    }

    async fn execute(self, _ctx: &Ctx) -> Result<SandboxTargetsResponse, CommandError> {
        Ok(SandboxTargetsResponse {
            items: sandbox_targets(),
        })
    }
}

inventory::submit! { CommandDescriptor::of::<ListSandboxTargets>() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_commands_are_part_of_the_core_surface() {
        assert_eq!(GetSessionSandbox::meta().required_feature(), None);
        assert_eq!(ListSandboxTargets::meta().required_feature(), None);
    }
}
