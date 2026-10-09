use super::queries as q;
use crate::domains::common::*;
use everruns_core::SessionResourceEntry;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListSessionResources {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "list_session_resources",
    category = "session_resources",
    description = "List all resources registered in a session.",
    method = "GET",
    path = "/v1/sessions/{session_id}/resources",
    positional = "session_id",
    cli = CliRoute::new(&["sessions", "resources"], "list").with_args(&[CliArg::new("session_id").at(1)]).with_examples(&[CliExample::new("List the files and other resources registered in a session", "everruns sessions resources list session_01h9",)]),
)]
impl Command for ListSessionResources {
    type Output = Vec<SessionResourceEntry>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SessionResourceEntry>, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::list_for_session(&ctx.db, ctx.org_id(), session_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session"))
    }
}
