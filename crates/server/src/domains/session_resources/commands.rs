use super::queries as q;
use crate::domains::common::*;
use everruns_core::SessionResourceEntry;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSessionResources {
    pub session_id: String,
}

impl Command for ListSessionResources {
    type Output = Vec<SessionResourceEntry>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_session_resources",
            category: "session_resources",
            description: "List all resources registered in a session.",
            method: "GET",
            path: "/v1/sessions/{session_id}/resources",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SessionResourceEntry>, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        crate::domains::session_resources::SessionResourceService::new(ctx.db.clone())
            .list_for_session(ctx.org_id(), session_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))
    }
}

inventory::submit! { CommandDescriptor::of::<ListSessionResources>() }
