use super::*;

// ============================================================================
// App State
// ============================================================================

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: Arc<EventService>,
    pub session_file_service: Arc<WorkspaceFileService>,
    pub session_sandbox_service: Option<Arc<SessionSandboxService>>,
    pub capability_service: Arc<CapabilityService>,
    pub connector_registry: everruns_contracts::connector::ConnectorRegistry,
    pub budget_service: Arc<BudgetService>,
    pub reporting_service: Arc<ReportingService>,
    pub runner: Arc<dyn TurnBackend>,
    pub auth: AuthState,
    pub org_rate_limiter: crate::auth::rate_limit::OrgRateLimiter,
    pub encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
    pub workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    /// Control-plane repository uses resolve scoped installation credentials here.
    pub connection_resolver:
        Option<Arc<dyn everruns_core::connection_services::UserConnectionResolver>>,
    pub fallback_base_harness_name: Option<String>,
    pub fallback_default_harness_name: Option<String>,
    pub sqldb_store: Option<Arc<dyn SessionSqlDbStore>>,
    /// System utility LLM and decisions for sanctioned internal commands.
    pub utility_llm_service: Arc<dyn everruns_core::UtilityLlmService>,
    pub decisions: Arc<dyn everruns_core::DecisionsService>,
    /// Agent health check service, so health-check commands work over MCP too.
    pub health_check_service: Option<Arc<crate::domains::agents::AgentHealthCheckService>>,
    /// Whole-install health checks, so `system doctor` works over MCP too.
    pub system_doctor: Option<Arc<crate::domains::system::doctor::SystemDoctor>>,
    pub slack_provisioner: Option<Arc<dyn SlackAppProvisioner>>,
    /// The channel host Slack runs on, for schedule triggers fired by hand
    /// that run in a channel conversation.
    pub channels: Option<Arc<crate::channels::host::ServerChannels>>,
    /// OAuth catalog preset connection check (`api::user_connections`).
    pub mcp_oauth_checker: Option<context::SharedMcpOAuthChecker>,
    /// Absolute URL of `/.well-known/oauth-protected-resource/mcp`, used to
    /// populate the `WWW-Authenticate: Bearer resource_metadata="..."` header
    /// on 401 responses per RFC 9728 §5.1 and the MCP 2025-06-18 auth spec.
    /// Path-derived per RFC 9728 §3.1 for the `/mcp` resource.
    /// `None` disables the header (e.g. tests without an issuer configured).
    pub resource_metadata_url: Option<String>,
    /// Canonical MCP resource URL (`{root}/mcp`) that MCP OAuth access tokens are
    /// bound to (RFC 8707 audience). The `McpAuthUser` extractor passes this to
    /// `validate_mcp_token` so only tokens minted for this exact resource are
    /// accepted (TM-MCP-006). `None` still rejects regular access tokens via the
    /// token_type check but cannot match an audience.
    pub mcp_resource: Option<String>,
    /// Public root (`{root}`) that URL mode elicitation pages are built from —
    /// the same root `/mcp` is served under. `None` disables the tools that
    /// elicit, since an elicitation with no reachable URL is worse than no tool.
    pub elicitation_base_url: Option<String>,
    /// Outbound MCP Events (EVE-1121). `None` leaves `events/*` undefined.
    pub mcp_events: Option<Arc<crate::domains::mcp_servers::events::McpEventsService>>,
    /// Provider-domain services; `None` leaves `sync_provider_models` unavailable.
    pub provider_services: Option<ProviderServices>,
    /// Downloads files sent by URL with `agent_run` / `session_send_message`.
    pub egress: Arc<dyn everruns_core::EgressService>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn TurnBackend>,
        auth: AuthState,
        host_composition: &HostComposition,
        built_in_harnesses: &[crate::domains::harnesses::record::BuiltInHarnessDefinition],
        event_delivery: crate::live_updates::event_delivery::EventDelivery,
        encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
        workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
        capability_service: Arc<CapabilityService>,
        sqldb_store: Option<Arc<dyn SessionSqlDbStore>>,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::with_registry(
                db.clone(),
                (*host_composition.capability_registry()).clone(),
            )),
            message_service: Arc::new(MessageService::new(
                db.clone(),
                runner.clone(),
                event_delivery.clone(),
            )),
            event_service: Arc::new(EventService::new(db.clone(), event_delivery)),
            session_file_service: Arc::new(WorkspaceFileService::new(db.clone())),
            egress: host_composition.egress_service(),
            session_sandbox_service: None,
            capability_service,
            // Hosted connector catalog (EVE-879): defaults to the OSS
            // inventory preset; server composition overrides via
            // `with_connector_registry`.
            connector_registry: crate::platform::oss_connector_registry(),
            budget_service: Arc::new(BudgetService::new(db.clone())),
            reporting_service: Arc::new(ReportingService::new(db.clone())),
            db,
            runner,
            auth,
            org_rate_limiter: crate::auth::rate_limit::OrgRateLimiter::default(),
            encryption,
            workflow_store,
            connection_resolver: None,
            fallback_base_harness_name: crate::domains::harnesses::record::harness_for_role(
                built_in_harnesses,
                crate::domains::harnesses::record::BuiltInHarnessRole::Base,
            )
            .map(|h| h.name.clone()),
            fallback_default_harness_name: crate::domains::harnesses::record::harness_for_role(
                built_in_harnesses,
                crate::domains::harnesses::record::BuiltInHarnessRole::Default,
            )
            .map(|h| h.name.clone()),
            sqldb_store,
            utility_llm_service: host_composition.utility_llm_service(),
            decisions: host_composition.decisions(),
            slack_provisioner: None,
            channels: None,
            mcp_oauth_checker: None,
            health_check_service: None,
            system_doctor: None,
            resource_metadata_url: None,
            mcp_resource: None,
            elicitation_base_url: None,
            mcp_events: None,
            provider_services: None,
        }
    }

    pub fn with_org_rate_limiter(
        mut self,
        limiter: crate::auth::rate_limit::OrgRateLimiter,
    ) -> Self {
        self.org_rate_limiter = limiter;
        self
    }

    /// Set the canonical MCP resource URL (`{root}/mcp`) used to validate the
    /// audience of MCP OAuth access tokens (TM-MCP-006).
    pub fn with_mcp_resource(mut self, resource: impl Into<String>) -> Self {
        self.mcp_resource = Some(resource.into());
        self
    }

    /// Public root that URL mode elicitation pages are built from.
    pub fn with_elicitation_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.elicitation_base_url = Some(base_url.into());
        self
    }

    pub fn with_virtual_registry(
        mut self,
        registry: Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>,
    ) -> Self {
        self.session_file_service =
            Arc::new(WorkspaceFileService::new(self.db.clone()).with_virtual_registry(registry));
        self
    }

    pub fn with_session_sandbox_service(mut self, service: Arc<SessionSandboxService>) -> Self {
        self.session_sandbox_service = Some(service);
        self
    }

    pub fn with_health_check_service(
        mut self,
        service: Arc<crate::domains::agents::AgentHealthCheckService>,
    ) -> Self {
        self.health_check_service = Some(service);
        self
    }

    pub fn with_system_doctor(
        mut self,
        doctor: Arc<crate::domains::system::doctor::SystemDoctor>,
    ) -> Self {
        self.system_doctor = Some(doctor);
        self
    }
}

impl_auth_state!(AppState);
