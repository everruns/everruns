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
// - The config is read from `injected:knowledge_base` when the session's
//   effective capabilities (harness, agent, session) list it, else from the
//   native `knowledge_base` entry, so an agent works the same before and
//   after it is converted.
// - Native tool names: the model keeps calling `search_knowledge`.

use super::InjectedCapabilities;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use everruns_capabilities::capabilities::{KNOWLEDGE_BASE_CAPABILITY_ID, KnowledgeBaseCapability};
use everruns_capabilities::{KnowledgeStore, KnowledgeStoreExt};
use everruns_contracts::CapabilityRef;
use everruns_contracts::runtime::capabilities::DeclarativeCapabilityDefinition;
use everruns_contracts::runtime::capabilities::injected::{
    InjectedAttach, InjectedCapabilityDefinition, InjectedServer, InjectedToolNames,
    injected_capability_id,
};
use everruns_contracts::typed_id::{OrgId, SessionId};
use everruns_core::capabilities::Capability;
use everruns_core::mcp::injected_server::{InjectedCapabilityService, VerifiedSession};
use everruns_core::merge_capabilities;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::Tool;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

pub const NAME: &str = KNOWLEDGE_BASE_CAPABILITY_ID;

/// Reads one capability's config for a session's agent.
#[async_trait]
pub trait SessionCapabilityConfig: Send + Sync {
    /// The config of the first of `capability_ids` among the session's
    /// effective capabilities (harness, agent and session layers). `None`
    /// when the session is not in `org` or none of them is enabled.
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
        let Some(row) = self.db.get_session(internal_org, session).await? else {
            return Ok(None);
        };
        // The session's effective capabilities, as its turns see them: the
        // harness chain, then an active agent's, then the session's own
        // (`merge_capabilities`, a later layer wins by id). Agentless sessions
        // and harness- or session-level bindings resolve too (codex review,
        // 2026-11-24: reading only the agent's rows returned None for them).
        let harness = match row.harness_id {
            Some(id) => {
                crate::domains::harnesses::queries::resolve_effective(&self.db, internal_org, id)
                    .await?
                    .map(|h| h.capabilities)
                    .unwrap_or_default()
            }
            None => vec![],
        };
        let agent = match row.agent_id {
            Some(id)
                if self
                    .db
                    .get_agent_config(internal_org, id)
                    .await?
                    .is_some_and(|a| a.status == "active") =>
            {
                self.db
                    .get_agent_capabilities(id.uuid())
                    .await?
                    .into_iter()
                    .map(|r| CapabilityRef::with_config(r.capability_id, r.config))
                    .collect()
            }
            _ => vec![],
        };
        let own: Vec<CapabilityRef> = serde_json::from_value(row.capabilities).unwrap_or_default();
        let effective = merge_capabilities(&merge_capabilities(&harness, &agent), &own);
        Ok(capability_ids.iter().find_map(|id| {
            effective
                .iter()
                .find(|c| c.capability_id() == *id)
                .map(|c| c.config_value().clone())
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

#[cfg(test)]
mod storage_config_tests {
    //! The session's effective config on a real database (codex review,
    //! 2026-11-24): agentless sessions and harness or session bindings.
    use super::*;
    use crate::storage::StorageBackend;
    use serde_json::json;

    const ORG: i64 = 2;

    async fn harness_with(
        db: &StorageBackend,
        caps: Vec<(String, i32, Value)>,
    ) -> everruns_contracts::typed_id::HarnessId {
        let id = db
            .create_harness(
                ORG,
                crate::storage::CreateHarnessRow {
                    localizations: Default::default(),
                    name: format!("Test harness {}", uuid::Uuid::new_v4().simple()),
                    display_name: None,
                    icon: None,
                    description: None,
                    intro_markdown: None,
                    short_description: None,
                    starters: json!([]),
                    system_prompt: Some("You are helpful.".to_string()),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: json!([]),
                    mcp_servers: json!({}),
                    is_built_in: false,
                    network_access: None,
                    embedder_metadata: json!({}),
                },
            )
            .await
            .unwrap()
            .id;
        db.set_harness_capabilities(id.uuid(), caps).await.unwrap();
        id
    }

    #[tokio::test]
    async fn an_agentless_sessions_harness_and_session_bindings_resolve() {
        let db = Arc::new(StorageBackend::test_database());
        let configs = StorageSessionCapabilityConfig::new(db.clone());
        let org: OrgId =
            everruns_contracts::runtime::organization::org_public_id_from_internal(ORG)
                .parse()
                .unwrap();
        let ids = [injected_capability_id(NAME).as_str(), NAME].map(str::to_owned);
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();

        let harness = harness_with(
            &db,
            vec![(
                NAME.to_string(),
                0,
                json!({"collections": ["from-harness"]}),
            )],
        )
        .await;
        let mut row = crate::storage::test_database::test_session_row(ORG);
        row.harness_id = Some(harness);
        let session = db.create_session(row).await.unwrap();
        assert_eq!(
            configs.config(org, session.id, &ids).await.unwrap(),
            Some(json!({"collections": ["from-harness"]})),
            "no agent: the harness's binding"
        );

        let mut row = crate::storage::test_database::test_session_row(ORG);
        row.harness_id = Some(harness);
        row.capabilities = serde_json::to_value(vec![CapabilityRef::with_config(
            NAME,
            json!({"collections": ["from-session"]}),
        )])
        .unwrap();
        let session = db.create_session(row).await.unwrap();
        let got = configs.config(org, session.id, &ids).await.unwrap();
        assert_eq!(
            got,
            Some(json!({"collections": ["from-session"]})),
            "the session's own binding wins"
        );

        let other: OrgId =
            everruns_contracts::runtime::organization::org_public_id_from_internal(3)
                .parse()
                .unwrap();
        assert_eq!(
            configs.config(other, session.id, &ids).await.unwrap(),
            None,
            "another org sees nothing"
        );
    }
}
