//! The worker's tool approval lookup: the session organization's approval
//! policy and the session owner's own approval settings.
//!
//! Decision: the owner's settings are sanitized here as well as validated on
//! write, so a stored value that predates validation can only add caution.
//! A session without a resolved owner (an API key, a channel) gets the org
//! policy alone. Failures are fixed text; the worker then falls back to the
//! agent's own config, which the org cap cannot loosen further.

use super::owned_session;
use crate::domains::common::*;
use crate::domains::sessions::SESSION_VIEW;
use everruns_contracts::tool_approval_policy::{
    PERSONAL_APPROVAL_PREFERENCE_KEY, PersonalToolApproval, SessionToolApprovalPolicy,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetToolApprovalPolicy {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "worker_get_tool_approval_policy",
    category = "connections",
    description = "Internal: the session organization's tool approval policy and the session owner's own approval settings.",
    method = "GET",
    path = "/internal/sessions/{session_id}/tool_approval/policy",
    policy = SESSION_VIEW
)]
impl Command for WorkerGetToolApprovalPolicy {
    type Output = SessionToolApprovalPolicy;

    async fn execute(self, ctx: &Ctx) -> Result<SessionToolApprovalPolicy, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        let failed = |error: anyhow::Error| {
            tracing::error!(%error, "Failed to load tool approval policy");
            CommandError::internal(anyhow::anyhow!("Failed to load tool approval policy"))
        };
        let org = ctx
            .db
            .org_tool_approval_policy(ctx.org_id())
            .await
            .map_err(failed)?;
        let owner = ctx
            .db
            .get_session(ctx.org_id(), session_id)
            .await
            .map_err(failed)?
            .and_then(|session| session.resolved_owner_user_id);
        let personal = match owner {
            Some(user_id) => ctx
                .db
                .get_user_preference(user_id, PERSONAL_APPROVAL_PREFERENCE_KEY)
                .await
                .map_err(failed)?
                .and_then(|row| serde_json::from_str::<PersonalToolApproval>(&row.value).ok())
                .map(PersonalToolApproval::sanitized)
                .unwrap_or_default(),
            None => PersonalToolApproval::default(),
        };
        Ok(SessionToolApprovalPolicy { org, personal })
    }
}
