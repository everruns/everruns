use super::*;

pub(crate) fn domain_context(caller: Caller, state: &AppState) -> crate::domains::common::Ctx {
    let mut ctx = crate::domains::common::Ctx::new(
        caller,
        state.db.clone(),
        state.capability_service.clone(),
        state.encryption.clone(),
        state.auth.permission_resolver.clone(),
    )
    .with_connector_registry(state.connector_registry.clone())
    .with_org_rate_limiter(state.org_rate_limiter.clone())
    .with_session_service(state.session_service.clone())
    .with_message_service(state.message_service.clone())
    .with_event_service(state.event_service.clone())
    .with_reporting_service(state.reporting_service.clone())
    .with_session_file_service(state.session_file_service.clone())
    .with_runner(state.runner.clone())
    .with_fallback_harness_name(state.fallback_default_harness_name.clone())
    .with_slack_provisioner(state.slack_provisioner.clone())
    .with_mcp_oauth_checker(state.mcp_oauth_checker.clone())
    .with_utility_llm_service(state.utility_llm_service.clone())
    .with_decisions(state.decisions.clone())
    .with_github_apps(github_apps(state));
    if let Some(service) = &state.health_check_service {
        ctx = ctx.with_health_check_service(service.clone());
    }
    if let Some(service) = &state.session_sandbox_service {
        ctx = ctx.with_session_sandbox_service(service.clone());
    }
    if let Some(store) = &state.sqldb_store {
        ctx = ctx.with_sqldb_store(store.clone());
    }
    ctx.channels = state.channels.clone();
    with_provider_services(ctx, state).with_workflow_store(state.workflow_store.clone())
}

pub(crate) fn mcp_ctx(org: &ResolvedOrg, state: &AppState) -> Ctx {
    let mut ctx = Ctx::new(
        Caller::from(org),
        state.db.clone(),
        state.capability_service.clone(),
        state.encryption.clone(),
        state.auth.permission_resolver.clone(),
    )
    .with_feature_flags(org.feature_flags.clone())
    .with_org_rate_limiter(state.org_rate_limiter.clone())
    .with_slack_provisioner(state.slack_provisioner.clone())
    .with_mcp_oauth_checker(state.mcp_oauth_checker.clone())
    .with_utility_llm_service(state.utility_llm_service.clone())
    .with_decisions(state.decisions.clone())
    .with_github_apps(github_apps(state));
    if let Some(service) = &state.health_check_service {
        ctx = ctx.with_health_check_service(service.clone());
    }
    with_provider_services(ctx, state)
}

/// Agent GitHub Apps with this server's public URLs, so a manifest started
/// over MCP or `/v1/commands` names the same URLs as one started over REST.
fn github_apps(state: &AppState) -> crate::github_apps::GitHubAppContext {
    crate::github_apps::GitHubAppContext::from_env().with_auth_config(&state.auth.config)
}

/// Attach the provider-domain services `/v1/providers` uses, so provider
/// commands behave the same over MCP and the generic command adapter.
fn with_provider_services(
    ctx: crate::domains::common::Ctx,
    state: &AppState,
) -> crate::domains::common::Ctx {
    match &state.provider_services {
        Some(services) => ctx.with_provider_services(services),
        None => ctx,
    }
}

/// The OAuth catalog preset connection check, implemented in
/// `api::user_connections` and handed to every command context.
pub type SharedMcpOAuthChecker =
    Arc<dyn crate::domains::mcp_servers::connection_check::McpOAuthConnectionChecker>;

impl AppState {
    pub fn with_provider_services(mut self, services: ProviderServices) -> Self {
        self.provider_services = Some(services);
        self
    }

    pub fn with_resource_metadata_url(mut self, url: impl Into<String>) -> Self {
        self.resource_metadata_url = Some(url.into());
        self
    }

    pub fn with_connector_registry(
        mut self,
        registry: everruns_contracts::connector::ConnectorRegistry,
    ) -> Self {
        self.connector_registry = registry;
        self
    }

    pub fn with_mcp_oauth_checker(mut self, checker: Option<SharedMcpOAuthChecker>) -> Self {
        self.mcp_oauth_checker = checker;
        self
    }

    pub fn with_slack_provisioner(
        mut self,
        provisioner: Option<
            Arc<
                dyn crate::domains::agent_channels::record::slack_provisioning::SlackAppProvisioner,
            >,
        >,
    ) -> Self {
        self.slack_provisioner = provisioner;
        self
    }

    pub fn with_mcp_events(
        mut self,
        service: Arc<crate::domains::mcp_servers::events::McpEventsService>,
    ) -> Self {
        self.mcp_events = Some(service);
        self
    }

    pub fn with_channels(
        mut self,
        channels: Option<Arc<crate::channels::host::ServerChannels>>,
    ) -> Self {
        self.channels = channels;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_imports::contracts::driver_registry::DriverRegistry;

    fn state_with_provider_services() -> AppState {
        let db = Arc::new(StorageBackend::test_database());
        let auth = AuthState::builtin(crate::auth::AuthConfig::default(), db.clone());
        let host = HostComposition::builder().build();
        let providers = crate::api::providers::AppState::new(
            db.clone(),
            None,
            Arc::new(DriverRegistry::new()),
            auth.clone(),
            None,
        );
        AppState::new(
            db.clone(),
            Arc::new(crate::channels::slack::events::tests_support::NoopRunner),
            auth,
            &host,
            &[],
            crate::live_updates::event_delivery::EventDelivery::in_memory(),
            None,
            None,
            Arc::new(CapabilityService::new(db, None)),
            None,
        )
        .with_provider_services(providers.provider_services())
    }

    fn assert_provider_services(ctx: &Ctx) {
        assert!(ctx.provider_service.is_some(), "provider service missing");
        assert!(
            ctx.model_sync_service.is_some(),
            "model sync service missing"
        );
        assert!(ctx.model_service.is_some(), "model service missing");
    }

    /// Provider commands dispatched over MCP (and the generic command adapter,
    /// which shares this context) need the services `/v1/providers` attaches.
    /// Without them `sync_provider_models` failed with "Model sync service not
    /// configured" (Sentry EVERRUNS-2D, EVERRUNS-1G) and provider
    /// create/update silently skipped model provisioning.
    #[tokio::test]
    async fn mcp_contexts_carry_provider_services() {
        let state = state_with_provider_services();
        let org = ResolvedOrg {
            org_id: everruns_core::DEFAULT_ORG_ID,
            public_id: "org_test".into(),
            name: "Test".into(),
            user_id: None,
            role: OrgRole::Owner,
            is_platform_user: false,
            feature_flags: Default::default(),
            read_only: false,
        };

        assert_provider_services(&domain_context(Caller::from(&org), &state));
        assert_provider_services(&mcp_ctx(&org, &state));
    }
}
