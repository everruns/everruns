use super::*;

impl DirectWorkerAdapters {
    pub(super) async fn invoke_scheduled_trigger(
        &self,
        org_id: i64,
        agent_id: &str,
        trigger_id: &str,
        invocation_id: uuid::Uuid,
    ) -> Result<serde_json::Value> {
        let runner = self
            .runner
            .clone()
            .ok_or_else(|| store_error("Agent runner not configured"))?;
        let session_service = if let Some(registry) = self.virtual_registry.clone() {
            SessionService::with_registry(self.db.clone(), self.capability_registry.clone())
                .with_virtual_registry(registry)
        } else {
            SessionService::with_registry(self.db.clone(), self.capability_registry.clone())
        };
        let message_service = MessageService::new(
            self.db.clone(),
            runner,
            self.event_service.event_delivery().clone(),
        );

        let result = crate::domains::agent_triggers::invoke_scheduled_agent_trigger(
            &self.db,
            &session_service,
            &message_service,
            self.channels.as_deref(),
            self.encryption.as_ref(),
            org_id,
            agent_id,
            trigger_id,
            invocation_id,
        )
        .await
        .map_err(|error| store_error(format!("Failed to invoke agent trigger: {error}")))?;

        Ok(serde_json::json!({
            "session_id": result.session_id.to_string(),
            "created_session": result.created_session,
        }))
    }
}
