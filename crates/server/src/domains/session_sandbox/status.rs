//! Provider status reads use the same pinned specification, current runtime
//! instance and credential resolver as the session's own Sandbox tools.

use everruns_capabilities::session_sandbox::{
    SessionSandboxStatusResponse, create_session_sandbox_provider, load_session_sandbox_state,
};
use everruns_contracts::typed_id::SessionId;

use super::SessionSandboxService;
use super::plane::PlaneSandboxStorage;
use crate::domains::session_storage::plane::PlaneStorage;

impl SessionSandboxService {
    pub(crate) async fn provider_status(
        &self,
        org_id: i64,
        session: SessionId,
    ) -> anyhow::Result<Option<SessionSandboxStatusResponse>> {
        let Some(config) = super::plane::config(&self.db, session).await? else {
            return Ok(None);
        };
        let context = match PlaneStorage::on(&self.db, org_id, session).await? {
            Some(storage) => {
                self.plane_tool_context(session, PlaneSandboxStorage::new(storage))
                    .await
            }
            None => self.tool_context(session).await,
        };
        let state = load_session_sandbox_state(&context)
            .await
            .map_err(|_| anyhow::anyhow!("Sandbox state unavailable"))?;
        let Some(state) = state else {
            return Ok(None);
        };
        anyhow::ensure!(
            state.provider == config.provider,
            "Sandbox provider changed"
        );
        let provider = create_session_sandbox_provider(&config.provider)
            .ok_or_else(|| anyhow::anyhow!("Sandbox provider unavailable"))?;
        let status = provider
            .status(&context, &config, &state)
            .await
            .map_err(|_| anyhow::anyhow!("Sandbox provider status unavailable"))?;
        // A replacement or a lifecycle operation during the read invalidates
        // its answer, including when the authoritative state is on a shard.
        let current = load_session_sandbox_state(&context)
            .await
            .map_err(|_| anyhow::anyhow!("Sandbox state unavailable"))?;
        if current.as_ref() != Some(&state) {
            return Ok(None);
        }
        anyhow::ensure!(
            status.external_id == state.instance.external_id && status.provider == state.provider,
            "Provider answered for another Sandbox"
        );
        Ok(Some(status))
    }
}
