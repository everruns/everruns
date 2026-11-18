//! What a tool on the data plane asks the control plane at call time (data
//! plane v2, step 11.6b).
//!
//! A hosted capability's tools look org answers up as they run: a session's
//! connection token, a GitHub token for one use, the agent's service API
//! key, the org's web search settings and account keys, the tool approval
//! policy. On a v1 worker they reach the control plane through internal
//! commands; here the same resolver ([`CommandConnectionResolver`]) runs the
//! same commands over the link: executor, directory cell, control plane
//! (`POST /dp/v1/tenants/{org}/lookups`). The answer comes back sealed with
//! the org's data key, so the cell that forwards it holds ciphertext only.
//!
//! Decisions:
//! - Only the commands in [`LOOKUP_COMMANDS`] go over; the rest stay with
//!   the data plane: sandbox keys are sealed org secrets the control plane
//!   wrote when it pinned the sandbox (`sandbox.rs`), MCP credentials are
//!   the org's MCP secrets (`mcp.rs`), and a cell leases nothing to clean
//!   up. Anything else is refused here, before it leaves the executor.
//! - A standalone cell has no control plane: its tools get what a host
//!   without org settings gives (no connections, default settings), as they
//!   did before lookups.
//! - A lookup that cannot reach the control plane fails the tool call, as
//!   on a v1 worker whose control plane is down: a credential or a policy
//!   never falls back to a default.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use everruns_contracts::dataplane::{LOOKUP_COMMANDS, LookupAnswer};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::web_search::WebSearchOrgSettings;
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_contracts::tool_approval_policy::SessionToolApprovalPolicy;
use everruns_contracts::typed_id::SessionId;
use everruns_core::connection_services::{
    GitHubAccess, McpResolvedCredential, ServiceApiKeyConnection, UserConnectionResolver,
};
use everruns_internal_protocol::commands::{CommandConnectionResolver, InternalCommandTransport};
use everruns_internal_protocol::proto;
use serde_json::Value;
use uuid::Uuid;

use crate::providers::OrgMaterial;

/// One org's lookups, as the internal command transport the shared
/// resolver runs over.
pub struct OrgLookups {
    material: Weak<dyn OrgMaterial>,
    org: String,
}

#[async_trait]
impl InternalCommandTransport for OrgLookups {
    async fn execute_internal_command(
        &self,
        name: &str,
        params: Value,
    ) -> Result<std::result::Result<Value, proto::CommandError>> {
        if !LOOKUP_COMMANDS.contains(&name) {
            return Ok(Err(proto::CommandError {
                kind: proto::command_error::Kind::Forbidden as i32,
                message: format!("{name} is not available on the data plane"),
            }));
        }
        let material = self
            .material
            .upgrade()
            .ok_or_else(|| AgentLoopError::store("the executor is stopping"))?;
        match material.lookup(&self.org, name, params).await {
            Ok(LookupAnswer::Value(value)) => Ok(Ok(value)),
            Ok(LookupAnswer::Error { kind, message }) => {
                Ok(Err(proto::CommandError { kind, message }))
            }
            Err(error) => {
                tracing::warn!(org = %self.org, command = name, %error, "lookup failed");
                Err(AgentLoopError::store(
                    "the control plane could not be reached for this lookup",
                ))
            }
        }
    }
}

/// The connection resolver of one activation of `session`: `keys` for its
/// sandbox, and the control plane's lookups when `material` reaches one.
pub fn connections(
    keys: Arc<dyn UserConnectionResolver>,
    material: Option<&Weak<dyn OrgMaterial>>,
    session: SessionId,
) -> Arc<dyn UserConnectionResolver> {
    let lookups = material.and_then(|weak| {
        let material = weak.upgrade()?;
        if !material.looks_up() {
            return None;
        }
        let org = material.tenant(session)?;
        let resolver: Arc<dyn UserConnectionResolver> =
            Arc::new(CommandConnectionResolver::new(OrgLookups {
                material: weak.clone(),
                org,
            }));
        Some(resolver)
    });
    match lookups {
        Some(lookups) => Arc::new(CellConnections { keys, lookups }),
        None => keys,
    }
}

/// Sandbox keys from the cell, every other lookup from the control plane.
struct CellConnections {
    keys: Arc<dyn UserConnectionResolver>,
    lookups: Arc<dyn UserConnectionResolver>,
}

#[async_trait]
impl UserConnectionResolver for CellConnections {
    fn for_execution(&self, input_message_id: Uuid) -> Option<Arc<dyn UserConnectionResolver>> {
        let lookups = self.lookups.for_execution(input_message_id)?;
        Some(Arc::new(Self {
            keys: self.keys.clone(),
            lookups,
        }))
    }

    async fn get_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<String>> {
        // MCP grants are the org's MCP secrets on the data plane (`mcp.rs`);
        // the control plane refuses them here too.
        if provider.starts_with("mcp_oauth_") {
            return Ok(None);
        }
        self.lookups
            .get_connection_token(session_id, provider)
            .await
    }

    async fn get_service_api_key_connection(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<ServiceApiKeyConnection>> {
        self.lookups
            .get_service_api_key_connection(session_id, provider)
            .await
    }

    async fn get_github_token(
        &self,
        session_id: SessionId,
        access: &GitHubAccess,
    ) -> Result<Option<McpResolvedCredential>> {
        self.lookups.get_github_token(session_id, access).await
    }

    async fn get_sandbox_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
        credential: &SessionSandboxCredential,
    ) -> Result<Option<String>> {
        self.keys
            .get_sandbox_connection_token(session_id, provider, credential)
            .await
    }

    async fn get_web_search_settings(
        &self,
        session_id: SessionId,
    ) -> Result<Option<WebSearchOrgSettings>> {
        self.lookups.get_web_search_settings(session_id).await
    }

    async fn get_tool_approval_policy(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionToolApprovalPolicy>> {
        self.lookups.get_tool_approval_policy(session_id).await
    }

    async fn get_web_search_account_key(
        &self,
        session_id: SessionId,
        account_id: Uuid,
    ) -> Result<Option<String>> {
        self.lookups
            .get_web_search_account_key(session_id, account_id)
            .await
    }
}

#[cfg(test)]
#[path = "lookups_tests.rs"]
mod tests;
