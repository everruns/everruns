// Knowledge Base as an injected capability: the `search_knowledge` tool,
// served by the control plane for whichever data plane runs the session.
//
// Design: knowledge/execution/injected-capabilities.md
//
// Decisions:
// - Reuses `KnowledgeBaseCapability`'s tool unchanged; this file only
//   supplies the session's config and the org-scoped store.
// - The session must belong to the token's group (organization). A session
//   id from another organization resolves to nothing, exactly like a missing
//   one, so the endpoint does not leak which sessions exist.
// - The agent's config is read from `injected:knowledge_base` when the agent
//   lists it, else from the native `knowledge_base` entry, so an agent works
//   the same before and after it is converted.
// - Native tool names: the model keeps calling `search_knowledge`.

use super::InjectedCapabilities;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use everruns_capabilities::capabilities::{KNOWLEDGE_BASE_CAPABILITY_ID, KnowledgeBaseCapability};
use everruns_capabilities::{KnowledgeStore, KnowledgeStoreExt};
use everruns_contracts::runtime::capabilities::DeclarativeCapabilityDefinition;
use everruns_contracts::runtime::capabilities::injected::{
    InjectedAttach, InjectedCapabilityDefinition, InjectedServer, InjectedToolNames,
    injected_capability_id,
};
use everruns_contracts::typed_id::{OrgId, SessionId};
use everruns_core::capabilities::Capability;
use everruns_core::mcp::injected_server::{InjectedCapabilityService, VerifiedSession};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::Tool;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

pub const NAME: &str = KNOWLEDGE_BASE_CAPABILITY_ID;

/// Reads one capability's config for a session's agent.
#[async_trait]
pub trait SessionCapabilityConfig: Send + Sync {
    /// The config of the first of `capability_ids` the session's agent
    /// lists. `None` when the session is not in `org` or its agent lists
    /// none of them.
    async fn config(
        &self,
        org: OrgId,
        session: SessionId,
        capability_ids: &[&str],
    ) -> Result<Option<Value>>;
}

/// [`SessionCapabilityConfig`] over the server's database.
pub struct StorageSessionCapabilityConfig {
    db: Arc<crate::storage::StorageBackend>,
}

impl StorageSessionCapabilityConfig {
    pub fn new(db: Arc<crate::storage::StorageBackend>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SessionCapabilityConfig for StorageSessionCapabilityConfig {
    async fn config(
        &self,
        org: OrgId,
        session: SessionId,
        capability_ids: &[&str],
    ) -> Result<Option<Value>> {
        let internal_org = everruns_core::organization::org_internal_id_from_public(org);
        let Some(session) = self.db.get_session(internal_org, session).await? else {
            return Ok(None);
        };
        let Some(agent_id) = session.agent_id else {
            return Ok(None);
        };
        let rows = self.db.get_agent_capabilities(agent_id.uuid()).await?;
        Ok(capability_ids.iter().find_map(|id| {
            rows.iter()
                .find(|row| row.capability_id == *id)
                .map(|row| row.config.clone())
        }))
    }
}

pub struct KnowledgeBaseService {
    definition: InjectedCapabilityDefinition,
    configs: Arc<dyn SessionCapabilityConfig>,
    store: Arc<dyn KnowledgeStore>,
}

impl KnowledgeBaseService {
    pub fn new(
        base_url: &str,
        configs: Arc<dyn SessionCapabilityConfig>,
        store: Arc<dyn KnowledgeStore>,
    ) -> Self {
        let capability = KnowledgeBaseCapability;
        let definition = InjectedCapabilityDefinition {
            capability: DeclarativeCapabilityDefinition {
                name: NAME.to_string(),
                display_name: Some(capability.name().to_string()),
                description: capability.description().to_string(),
                icon: capability.icon().map(str::to_string),
                category: capability.category().map(str::to_string),
                system_prompt: capability.system_prompt_addition().map(str::to_string),
                features: capability
                    .features()
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                risk_level: capability.risk_level(),
                ..DeclarativeCapabilityDefinition::default()
            },
            server: InjectedServer {
                url: InjectedCapabilities::url_for(base_url, NAME),
                tool_timeout_ms: None,
            },
            attach: InjectedAttach::OnRequest,
            match_labels: BTreeMap::new(),
            tools: InjectedToolNames::Native,
        };
        Self {
            definition,
            configs,
            store,
        }
    }

    fn org(session: &VerifiedSession) -> Result<OrgId> {
        session
            .claims
            .grp
            .as_deref()
            .ok_or_else(|| anyhow!("session token has no organization"))?
            .parse()
            .map_err(|_| anyhow!("session token group is not an organization"))
    }

    async fn session_config(&self, session: &VerifiedSession) -> Result<Option<Value>> {
        let injected = injected_capability_id(NAME);
        self.configs
            .config(
                Self::org(session)?,
                SessionId::from_uuid(session.session_id),
                &[injected.as_str(), KNOWLEDGE_BASE_CAPABILITY_ID],
            )
            .await
    }
}

#[async_trait]
impl InjectedCapabilityService for KnowledgeBaseService {
    fn definition(&self) -> &InjectedCapabilityDefinition {
        &self.definition
    }

    async fn tools(&self, session: &VerifiedSession) -> Result<Vec<Box<dyn Tool>>> {
        // A session whose agent does not use Knowledge Base gets no tools.
        Ok(match self.session_config(session).await? {
            Some(config) => KnowledgeBaseCapability.tools_with_config(&config),
            None => Vec::new(),
        })
    }

    async fn tool_context(&self, session: &VerifiedSession) -> Result<ToolContext> {
        Ok(ToolContext::new(SessionId::from_uuid(session.session_id))
            .with_org_id(Self::org(session)?)
            .with_extension(Arc::new(KnowledgeStoreExt(self.store.clone()))))
    }
}
