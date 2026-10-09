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
    .with_utility_llm_service(state.utility_llm_service.clone());
    if let Some(service) = &state.health_check_service {
        ctx = ctx.with_health_check_service(service.clone());
    }
    if let Some(service) = &state.session_sandbox_service {
        ctx = ctx.with_session_sandbox_service(service.clone());
    }
    if let Some(store) = &state.sqldb_store {
        ctx = ctx.with_sqldb_store(store.clone());
    }
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
    .with_utility_llm_service(state.utility_llm_service.clone());
    if let Some(service) = &state.health_check_service {
        ctx = ctx.with_health_check_service(service.clone());
    }
    with_provider_services(ctx, state)
}

/// Attach the provider-domain services `/v1/providers` uses, so provider
/// commands behave the same over MCP and the generic command adapter.
fn with_provider_services(
    mut ctx: crate::domains::common::Ctx,
    state: &AppState,
) -> crate::domains::common::Ctx {
    if let Some(services) = &state.provider_services {
        ctx = ctx
            .with_provider_service(services.provider.clone())
            .with_model_sync_service(services.model_sync.clone())
            .with_model_service(services.model.clone());
    }
    ctx
}

/// Provider-domain services shared with the `/v1/providers` routes. Shared
/// rather than rebuilt so the provider resolver whose cache they invalidate is
/// the one the runtime reads.
#[derive(Clone)]
pub struct ProviderServices {
    pub provider: Arc<crate::domains::providers::ProviderService>,
    pub model_sync: Arc<crate::services::ModelSyncService>,
    pub model: Arc<crate::domains::models::ModelService>,
}

impl From<&crate::api::providers::AppState> for ProviderServices {
    fn from(state: &crate::api::providers::AppState) -> Self {
        Self {
            provider: state.service.clone(),
            model_sync: state.sync_service.clone(),
            model: state.model_service.clone(),
        }
    }
}

impl AppState {
    pub fn with_provider_services(mut self, services: ProviderServices) -> Self {
        self.provider_services = Some(services);
        self
    }

    pub fn with_slack_provisioner(
        mut self,
        provisioner: Option<Arc<dyn crate::records::slack_provisioning::SlackAppProvisioner>>,
    ) -> Self {
        self.slack_provisioner = provisioner;
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
            false,
            crate::event_delivery::EventDelivery::in_memory(),
            None,
            None,
            Arc::new(CapabilityService::new(db, None)),
            None,
        )
        .with_provider_services((&providers).into())
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
        };

        assert_provider_services(&domain_context(Caller::from(&org), &state));
        assert_provider_services(&mcp_ctx(&org, &state));
    }
}
