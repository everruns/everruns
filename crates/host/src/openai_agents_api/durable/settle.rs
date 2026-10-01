//! Permanent provider failures (EVE-1126): a failure no retry fixes ends the
//! turn with a stable code instead of a retried activity error. See
//! `crate::openai_agents_api::lifecycle` for the classification.

use super::{AgentsApiTurnOutcome, Run, store_error};
use crate::openai_agents_api::AgentsApiError;

impl Run<'_> {
    /// A provider failure no retry fixes ends the turn with a stable code
    /// (see [`super::lifecycle`]); anything else stays an error for the
    /// durable engine to retry. A provider session that no longer exists is
    /// released, so the next turn starts a new one instead of failing again;
    /// the store queues the dropped id for deletion, which finds it gone.
    pub(super) async fn settle_permanent_failure(
        &mut self,
        error: AgentsApiError,
    ) -> Result<AgentsApiTurnOutcome, AgentsApiError> {
        let has_session = self.checkpoint.provider_session_id.is_some();
        let Some(failure) = crate::openai_agents_api::lifecycle::classify(&error, has_session)
        else {
            return Err(error);
        };
        if failure.code == crate::openai_agents_api::lifecycle::PROVIDER_SESSION_UNAVAILABLE {
            // A 404 may name a turn or an item; only a missing session
            // releases the session.
            let session_id = self.provider_session()?;
            match self.driver.client.retrieve_session(&session_id).await {
                Err(AgentsApiError::Api { status: 404, .. }) => {}
                _ => return Err(error),
            }
            tracing::warn!(
                session_id = %self.request.session_id,
                "Agents API provider session no longer exists; releasing it"
            );
            self.checkpoint.release_provider_session();
        } else {
            tracing::warn!(
                session_id = %self.request.session_id,
                code = failure.code,
                "Agents API turn failed permanently"
            );
        }
        let outcome = AgentsApiTurnOutcome::Failed {
            code: Some(failure.code.to_string()),
            message: failure.message,
            policy: false,
        };
        self.turn_mut().outcome = Some(serde_json::to_value(&outcome).map_err(store_error)?);
        self.save().await?;
        self.release().await?;
        Ok(outcome)
    }
}
