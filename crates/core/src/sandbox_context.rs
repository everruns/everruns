//! Runtime adapter for the provider-neutral managed sandbox service interface.

use async_trait::async_trait;
use everruns_contracts::session_sandbox::{SessionSandboxContext, SessionSandboxLease};
use everruns_contracts::tools::ToolExecutionResult;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::tool_context::ToolContext;

#[async_trait]
impl SessionSandboxContext for ToolContext {
    fn session_id(&self) -> crate::typed_id::SessionId {
        self.session_id
    }

    fn clone_context(&self) -> Arc<dyn SessionSandboxContext> {
        Arc::new(self.clone())
    }

    async fn connection_token(
        &self,
        provider: &str,
    ) -> Result<Option<String>, ToolExecutionResult> {
        match &self.connection_resolver {
            Some(resolver) => resolver
                .get_connection_token(self.session_id, provider)
                .await
                .map_err(ToolExecutionResult::internal_error),
            None => Ok(None),
        }
    }

    async fn resource_labels(&self) -> serde_json::Map<String, Value> {
        let mut labels = serde_json::Map::new();
        labels.insert("everruns".to_string(), json!("true"));
        labels.insert(
            "everruns.session_id".to_string(),
            json!(self.session_id.to_string()),
        );
        if let Some(store) = &self.session_store
            && let Ok(Some(session)) = store.get_session(self.session_id).await
        {
            labels.insert(
                "everruns.harness_id".to_string(),
                json!(session.harness_id.to_string()),
            );
            labels.insert(
                "everruns.org_id".to_string(),
                json!(session.organization_id),
            );
            if let Some(agent_id) = session.agent_id {
                labels.insert("everruns.agent_id".to_string(), json!(agent_id.to_string()));
            }
        }
        labels
    }

    async fn refresh_lease(&self, lease: SessionSandboxLease) -> Result<(), ToolExecutionResult> {
        let Some(store) = &self.leased_resource_store else {
            return Ok(());
        };
        let owner_user_id = match &self.connection_resolver {
            Some(resolver) => resolver
                .get_connection_user(self.session_id, &lease.provider)
                .await
                .ok()
                .flatten(),
            None => None,
        };
        store
            .upsert_resource(crate::UpsertLeasedResource {
                session_id: self.session_id,
                provider: lease.provider,
                resource_type: "sandbox".to_string(),
                external_id: lease.external_id,
                display_name: lease.display_name,
                owner_user_id,
                lease_duration_seconds: lease.duration_seconds,
                metadata: lease.metadata,
            })
            .await
            .map_err(ToolExecutionResult::internal_error)?;
        Ok(())
    }

    async fn release_lease(
        &self,
        provider: &str,
        external_id: &str,
    ) -> Result<(), ToolExecutionResult> {
        let Some(store) = &self.leased_resource_store else {
            return Ok(());
        };
        store
            .release_resource(self.session_id, provider, "sandbox", external_id)
            .await
            .map_err(ToolExecutionResult::internal_error)?;
        Ok(())
    }
}
