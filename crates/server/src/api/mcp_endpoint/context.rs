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
    ctx.with_workflow_store(state.workflow_store.clone())
}

impl AppState {
    pub fn with_slack_provisioner(
        mut self,
        provisioner: Option<Arc<dyn crate::records::slack_provisioning::SlackAppProvisioner>>,
    ) -> Self {
        self.slack_provisioner = provisioner;
        self
    }
}
