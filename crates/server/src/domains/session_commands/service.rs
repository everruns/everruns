// Session command service.
//
// Decision: system commands dispatch uniformly through
// `Capability::execute_command`. Context-aware commands like /btw are
// implemented inside their capability against the `CommandHost` facilities
// (see specs/commands.md); this service only wires the store-backed host
// from the worker adapters and routes the request.

use crate::direct_worker_adapters::DirectWorkerAdapters;
use crate::domains::mcp_servers::McpServerService;
use crate::errors::{BadRequestError, ResourceNotFoundError};
use crate::services::{EventService, LlmResolverService};
use crate::storage::StorageBackend;
use anyhow::Result;
use everruns_core::command::{
    CommandDescriptor, CommandExecutionContext, CommandResult, ExecuteCommandRequest,
};
use everruns_core::command_host::StoreCommandHost;
use everruns_core::runtime_context::resolve_runtime_capabilities;
use everruns_core::traits::{AgentStore, HarnessStore, SessionStore};
use everruns_core::typed_id::SessionId;
use everruns_core::{Agent, AgentLoopError, Caller, CapabilityRegistry, DriverRegistry, Harness};
use everruns_worker::worker_adapters::{
    AdapterAgentStore, AdapterHarnessStore, AdapterImageResolver, AdapterLlmProviderStore,
    AdapterMessageRetriever, AdapterSessionFileStore, AdapterSessionStore,
};
use std::collections::HashSet;
use std::sync::Arc;

pub struct SessionCommandService {
    db: Arc<StorageBackend>,
    event_service: Arc<EventService>,
    llm_resolver: Arc<LlmResolverService>,
    mcp_server_service: Arc<McpServerService>,
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    sqldb_store: everruns_core::traits::SessionSqlDbStoreRef,
    virtual_registry:
        Option<Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>>,
}

impl SessionCommandService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<StorageBackend>,
        event_service: Arc<EventService>,
        llm_resolver: Arc<LlmResolverService>,
        mcp_server_service: Arc<McpServerService>,
        capability_registry: CapabilityRegistry,
        driver_registry: DriverRegistry,
        sqldb_store: everruns_core::traits::SessionSqlDbStoreRef,
    ) -> Self {
        Self {
            db,
            event_service,
            llm_resolver,
            mcp_server_service,
            capability_registry,
            driver_registry,
            sqldb_store,
            virtual_registry: None,
        }
    }

    pub fn with_virtual_registry(
        mut self,
        registry: Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>,
    ) -> Self {
        self.virtual_registry = Some(registry);
        self
    }

    pub async fn list_system_commands(
        &self,
        caller: &Caller,
        session_id: SessionId,
    ) -> Result<Vec<CommandDescriptor>> {
        let (harness_chain, agent, session) = self
            .load_session_components(caller.org_id, session_id)
            .await?;
        let resolved = resolve_runtime_capabilities(
            &harness_chain,
            agent.as_ref(),
            &session,
            &self.capability_registry,
        );

        let mut seen = HashSet::new();
        let mut commands = Vec::new();
        for config in resolved.resolved_capability_configs {
            let Some(capability) = self.capability_registry.get(config.capability_id()) else {
                continue;
            };
            for command in capability.commands() {
                if seen.insert(command.name.clone()) {
                    commands.push(command);
                }
            }
        }

        Ok(commands)
    }

    pub async fn execute(
        &self,
        caller: &Caller,
        session_id: SessionId,
        req: ExecuteCommandRequest,
    ) -> Result<CommandResult> {
        let (harness_chain, agent, session) = self
            .load_session_components(caller.org_id, session_id)
            .await?;
        let resolved = resolve_runtime_capabilities(
            &harness_chain,
            agent.as_ref(),
            &session,
            &self.capability_registry,
        );

        // First capability declaring the name wins, matching the
        // deduplication order of `list_system_commands`.
        let capability = resolved
            .resolved_capability_configs
            .iter()
            .filter_map(|config| self.capability_registry.get(config.capability_id()))
            .find(|capability| {
                capability
                    .commands()
                    .iter()
                    .any(|command| command.name == req.name)
            })
            .cloned()
            .ok_or_else(|| {
                BadRequestError::new(format!(
                    "Unknown or unavailable system command: /{}",
                    req.name
                ))
            })?;

        let ctx = CommandExecutionContext::new(
            session_id,
            Arc::new(self.command_host(caller.org_id, session_id)),
        );
        capability
            .execute_command(&req, &ctx)
            .await
            .map_err(|error| match error {
                // Capability-level validation errors (missing argument,
                // unknown model override) stay HTTP 400; everything else is
                // an internal error. Provider failures never reach here —
                // capabilities return them as classified `success: false`.
                AgentLoopError::Configuration(message) => BadRequestError::new(message).into(),
                other => anyhow::Error::from(other),
            })
    }

    fn adapters(&self) -> DirectWorkerAdapters {
        let mut adapters = DirectWorkerAdapters::new(
            self.db.clone(),
            self.event_service.clone(),
            self.llm_resolver.clone(),
            self.mcp_server_service.clone(),
            self.capability_registry.clone(),
            self.driver_registry.clone(),
            self.sqldb_store.clone(),
        );
        if let Some(registry) = &self.virtual_registry {
            adapters = adapters.with_virtual_registry(registry.clone());
        }
        adapters
    }

    fn command_host(&self, org_id: i64, session_id: SessionId) -> StoreCommandHost {
        let adapters = self.adapters();
        StoreCommandHost::new(
            session_id,
            Arc::new(AdapterHarnessStore::new(adapters.clone(), org_id)),
            Arc::new(AdapterAgentStore::new(adapters.clone(), org_id)),
            Arc::new(AdapterSessionStore::new(adapters.clone(), org_id)),
            Arc::new(AdapterMessageRetriever::new(adapters.clone())),
            Arc::new(AdapterLlmProviderStore::new(adapters.clone(), org_id)),
            self.capability_registry.clone(),
            self.driver_registry.clone(),
        )
        .with_image_resolver(Arc::new(AdapterImageResolver::new(
            adapters.clone(),
            org_id,
        )))
        .with_file_store(Arc::new(AdapterSessionFileStore::new(adapters)))
    }

    async fn load_session_components(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<(Vec<Harness>, Option<Agent>, everruns_core::Session)> {
        let adapters = self.adapters();
        let session_store = AdapterSessionStore::new(adapters.clone(), org_id);
        let harness_store = AdapterHarnessStore::new(adapters.clone(), org_id);
        let agent_store = AdapterAgentStore::new(adapters, org_id);

        let session = session_store
            .get_session(session_id)
            .await?
            .ok_or(ResourceNotFoundError::new("Session"))?;
        let harness_chain = harness_store.get_harness_chain(session.harness_id).await?;
        if harness_chain.is_empty() {
            return Err(ResourceNotFoundError::new("Harness").into());
        }
        let agent = match session.agent_id {
            Some(agent_id) => agent_store.get_agent(agent_id).await?,
            None => None,
        };

        Ok((harness_chain, agent, session))
    }
}
