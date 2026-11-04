//! Runtime adapter for the provider-neutral managed sandbox service interface.

use crate::session_sandbox::{
    SessionSandboxContext, SessionSandboxCredential, SessionSandboxLease,
};
use crate::tools::ToolExecutionResult;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::runtime::tool_context::ToolContext;
#[async_trait]
impl SessionSandboxContext for ToolContext {
    fn session_id(&self) -> crate::runtime::typed_id::SessionId {
        self.session_id
    }

    fn clone_context(&self) -> Arc<dyn SessionSandboxContext> {
        Arc::new(self.clone())
    }

    fn network_access(&self) -> Option<crate::runtime::network_access::NetworkAccessList> {
        self.network_access.clone()
    }

    fn sandbox_proxy_credential(
        &self,
        valid_for: std::time::Duration,
    ) -> Result<Option<crate::runtime::SandboxProxyCredential>, ToolExecutionResult> {
        // The turn's egress service is scoped to this org and session, so
        // the credential names them whatever a tool asks for.
        let Some(egress) = &self.egress_service else {
            return Ok(None);
        };
        egress
            .sandbox_proxy_credential(None, valid_for)
            .map_err(|error| ToolExecutionResult::tool_error(error.to_string()))
    }

    async fn connection_token(
        &self,
        provider: &str,
    ) -> Result<Option<String>, ToolExecutionResult> {
        let Some(resolver) = &self.connection_resolver else {
            return Ok(None);
        };
        if provider == "github" {
            // THREAT[TM-GITHUB-006]: GitHub goes through the one declared-use
            // request: the person when they connected GitHub, else the agent.
            // A sandbox may call the API as well as git, and which repositories
            // it needs is not known here, so the token is not narrowed yet;
            // sandbox repositories on the template narrow it (github-model step 6).
            let access = crate::runtime::GitHubAccess::new(
                crate::runtime::mcp_server::McpServerActsAs::UserOrService,
                &[],
            );
            return resolver
                .get_github_token(self.session_id, &access)
                .await
                .map(|credential| credential.map(|credential| credential.token))
                .map_err(ToolExecutionResult::internal_error);
        }
        let token = resolver
            .get_connection_token(self.session_id, provider)
            .await;
        token.map_err(ToolExecutionResult::internal_error)
    }

    async fn sandbox_connection_token(
        &self,
        provider: &str,
        credential: &SessionSandboxCredential,
    ) -> Result<Option<String>, ToolExecutionResult> {
        let Some(resolver) = &self.connection_resolver else {
            return Ok(None);
        };
        let token = resolver
            .get_sandbox_connection_token(self.session_id, provider, credential)
            .await;
        token.map_err(ToolExecutionResult::internal_error)
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

    async fn get_provider_secret(&self, name: &str) -> Result<Option<String>, ToolExecutionResult> {
        let store = self.storage_store.as_ref().ok_or_else(|| {
            ToolExecutionResult::internal_error_msg("Session secret storage is unavailable")
        })?;
        store
            .get_secret(self.session_id, name)
            .await
            .map_err(ToolExecutionResult::internal_error)
    }

    async fn set_provider_secret(
        &self,
        name: &str,
        value: &str,
    ) -> Result<(), ToolExecutionResult> {
        let store = self.storage_store.as_ref().ok_or_else(|| {
            ToolExecutionResult::internal_error_msg("Session secret storage is unavailable")
        })?;
        store
            .set_secret(self.session_id, name, value)
            .await
            .map_err(ToolExecutionResult::internal_error)
    }

    async fn delete_provider_secret(&self, name: &str) -> Result<(), ToolExecutionResult> {
        let Some(store) = &self.storage_store else {
            return Ok(());
        };
        store
            .delete_secret(self.session_id, name)
            .await
            .map(|_| ())
            .map_err(ToolExecutionResult::internal_error)
    }

    async fn refresh_lease(&self, lease: SessionSandboxLease) -> Result<(), ToolExecutionResult> {
        let Some(store) = &self.leased_resource_store else {
            return Ok(());
        };
        let owner_user_id = match lease.credential.virtual_user_id {
            Some(owner) => Some(owner),
            None => match &self.connection_resolver {
                Some(resolver) => resolver
                    .get_connection_user(self.session_id, &lease.provider)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            },
        };
        let connection_id = lease.credential.connection_id;
        store
            .upsert_resource(crate::runtime::UpsertLeasedResource {
                session_id: self.session_id,
                provider: lease.provider,
                resource_type: "sandbox".to_string(),
                external_id: lease.external_id,
                display_name: lease.display_name,
                owner_user_id,
                connection_id,
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
