// Session command service.
//
// Decision: system commands dispatch uniformly through
// `Capability::execute_command`. Context-aware commands like /btw are
// implemented inside their capability against the `CommandHost` facilities
// (see knowledge/project/commands.md); this service only wires the store-backed host
// from the worker adapters and routes the request.

use crate::direct_worker_adapters::DirectWorkerAdapters;
use crate::domains::mcp_servers::McpServerService;
use crate::errors::{BadRequestError, ResourceNotFoundError};
use crate::kernel_imports::{
    AgentDefinition, Caller, CapabilityRegistry,
    everruns_provider::driver_registry::DriverRegistry, everruns_provider::error::AgentLoopError,
};
use crate::services::{EventService, ProviderResolverService};
use crate::storage::StorageBackend;
use anyhow::Result;
use everruns_core::command::{
    CommandDescriptor, CommandExecutionContext, CommandResult, ExecuteCommandRequest,
};
use everruns_core::execution_loading::AgentStore;
use everruns_core::runtime_context::resolve_runtime_capabilities;
use everruns_host::StoreCommandHost;
use everruns_platform::Harness;
use everruns_provider::typed_id::SessionId;
use everruns_worker::worker_adapters::{OrgAdapter, SessionAdapter, WorkerAdapters};
use std::collections::HashSet;
use std::sync::Arc;

pub struct SessionCommandService {
    db: Arc<StorageBackend>,
    event_service: Arc<EventService>,
    provider_resolver: Arc<ProviderResolverService>,
    mcp_server_service: Arc<McpServerService>,
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    sqldb_store: std::sync::Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore>,
    virtual_registry:
        Option<Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>>,
}

impl SessionCommandService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<StorageBackend>,
        event_service: Arc<EventService>,
        provider_resolver: Arc<ProviderResolverService>,
        mcp_server_service: Arc<McpServerService>,
        capability_registry: CapabilityRegistry,
        driver_registry: DriverRegistry,
        sqldb_store: std::sync::Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore>,
    ) -> Self {
        Self {
            db,
            event_service,
            provider_resolver,
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
        let (harness, agent, session) = self
            .load_session_components(caller.org_id, session_id)
            .await?;
        authorize_platform_chat_owner(caller, &harness, &session)?;
        let harness_definition = harness.definition();
        // EVE-882: capability resolution consumes the portable execution view.
        let execution_session = session.execution_session();
        let resolved = resolve_runtime_capabilities(
            &harness_definition,
            agent.as_ref(),
            &execution_session,
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
        let (harness, agent, session) = self
            .load_session_components(caller.org_id, session_id)
            .await?;
        authorize_platform_chat_owner(caller, &harness, &session)?;
        let harness_definition = harness.definition();
        // EVE-882: capability resolution consumes the portable execution view.
        let execution_session = session.execution_session();
        let resolved = resolve_runtime_capabilities(
            &harness_definition,
            agent.as_ref(),
            &execution_session,
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
            self.provider_resolver.clone(),
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
            Arc::new(OrgAdapter::new(adapters.clone(), org_id)),
            Arc::new(OrgAdapter::new(adapters.clone(), org_id)),
            Arc::new(OrgAdapter::new(adapters.clone(), org_id)),
            Arc::new(SessionAdapter::new(adapters.clone())),
            Arc::new(OrgAdapter::new(adapters.clone(), org_id)),
            self.capability_registry.clone(),
            self.driver_registry.clone(),
        )
        .with_image_resolver(Arc::new(OrgAdapter::new(adapters.clone(), org_id)))
        .with_file_store(Arc::new(SessionAdapter::new(adapters)))
    }

    async fn load_session_components(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<(Harness, Option<AgentDefinition>, everruns_platform::Session)> {
        let adapters = self.adapters();
        let agent_store = OrgAdapter::new(adapters.clone(), org_id);

        // Stored record (EVE-882): owner authorization is a platform surface,
        // so it reads the persisted Session rather than the execution view.
        let session = adapters
            .get_stored_session(org_id, session_id.uuid())
            .await?
            .ok_or(ResourceNotFoundError::new("Session"))?;
        // Stored (pre-merged) record: command listing/authorization are
        // status-agnostic platform surfaces, so they read the record rather
        // than the execution-validated definition (EVE-881).
        let harness = adapters
            .get_harness(org_id, session.harness_id.uuid())
            .await?
            .ok_or(ResourceNotFoundError::new("Harness"))?;
        let agent = match session.agent_id {
            Some(agent_id) => agent_store.get_agent(agent_id).await?,
            None => None,
        };

        Ok((harness, agent, session))
    }
}

fn authorize_platform_chat_owner(
    caller: &Caller,
    harness: &Harness,
    session: &everruns_platform::Session,
) -> Result<()> {
    // The adapters return the pre-merged record whose identity/built-in flag
    // are leaf-owned, matching the historical single-element chain check.
    let is_platform_chat = harness.is_built_in && harness.name == "platform-chat";
    if !crate::domains::sessions::platform_chat_owner_matches(caller, session, is_platform_chat) {
        return Err(
            everruns_core::PolicyError::denied("platform_chat_owner", "session owner").into(),
        );
    }
    Ok(())
}
