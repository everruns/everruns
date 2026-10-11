use super::*;

impl DirectPlatformStore {
    pub(super) async fn command_ctx(
        &self,
    ) -> everruns_contracts::error::Result<crate::domains::common::Ctx> {
        let caller = self.resolve_caller().await?;
        let feature_flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            self.org_id,
            &crate::records::FeatureFlagPolicy::current(),
        )
        .await
        .map_err(|error| {
            store_error(format!("Failed to resolve platform feature flags: {error}"))
        })?;
        let mut ctx = crate::domains::common::Ctx::new(
            caller,
            self.db.clone(),
            self.capability_service.clone(),
            self.encryption.clone(),
            self.permission_resolver.clone(),
        )
        .with_slack_provisioner(self.slack_provisioner.clone())
        .with_feature_flags(feature_flags)
        .with_connector_registry(self.connector_registry.clone())
        .with_workflow_store(self.workflow_store.clone())
        .with_session_service(self.session_service.clone())
        // wait_for_idle probes terminal turn events through list_events; without
        // the event service every subagent wait fails in the in-process adapter.
        .with_event_service(self.event_service.clone());

        if let Some(message_service) = &self.message_service {
            ctx = ctx.with_message_service(message_service.clone());
        }
        if let Some(runner) = &self.runner {
            ctx = ctx.with_runner(runner.clone());
        }

        Ok(ctx)
    }

    /// Run a runtime-view mutation a built-in tool makes from this session.
    /// It records like the gRPC worker path: through the session, with the
    /// tool's own reason.
    pub(super) async fn execute_runtime_command<T>(
        &self,
        name: &str,
        params: serde_json::Value,
        reason: &str,
    ) -> everruns_contracts::error::Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let intent = crate::domains::change_history::agent_session_intent(
            &self.db,
            self.org_id,
            self.session_id,
            crate::domains::change_history::ChangeSurface::Worker,
        )
        .await
        .with_reason(Some(reason.to_string()));
        let ctx = self.command_ctx().await?.with_change_intent(intent);
        let json =
            crate::worker_link::runtime_command_view::dispatch_runtime_view(name, params, &ctx)
                .await
                .map_err(|e| store_error(format!("Command {name} failed: {e}")))?;
        serde_json::from_str(&json)
            .map_err(|e| store_error(format!("Failed to decode {name} response: {e}")))
    }
}
