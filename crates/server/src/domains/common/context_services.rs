use super::*;

impl Ctx {
    pub fn with_feature_flags(mut self, feature_flags: FeatureFlags) -> Self {
        self.feature_flags = feature_flags;
        self
    }

    pub fn with_driver_registry(mut self, driver_registry: Arc<DriverRegistry>) -> Self {
        self.driver_registry = driver_registry;
        self
    }

    pub fn with_connector_registry(
        mut self,
        connector_registry: everruns_contracts::connector::ConnectorRegistry,
    ) -> Self {
        self.connector_registry = Some(connector_registry);
        self
    }

    pub fn with_org_rate_limiter(
        mut self,
        limiter: crate::auth::rate_limit::OrgRateLimiter,
    ) -> Self {
        self.org_rate_limiter = Some(limiter);
        self
    }

    pub fn with_session_service(
        mut self,
        service: Arc<crate::domains::sessions::SessionService>,
    ) -> Self {
        self.session_service = Some(service);
        self
    }

    pub fn with_message_service(
        mut self,
        service: Arc<crate::domains::messages::MessageService>,
    ) -> Self {
        self.message_service = Some(service);
        self
    }

    /// See [`Ctx::connection_resolver`].
    pub fn with_connection_resolver(
        mut self,
        resolver: Option<Arc<dyn everruns_core::connection_services::UserConnectionResolver>>,
    ) -> Self {
        self.connection_resolver = resolver;
        self
    }

    pub fn with_event_service(mut self, service: Arc<crate::services::EventService>) -> Self {
        self.event_service = Some(service);
        self
    }

    /// Declare the session whose agent runtime this caller is. See
    /// [`Ctx::acting_for_session`].
    pub fn acting_for_session(mut self, session_id: SessionId) -> Self {
        self.acting_for_session = Some(session_id);
        self
    }

    /// See [`Ctx::change_intent`].
    pub fn with_change_intent(
        mut self,
        intent: crate::domains::change_history::ChangeIntent,
    ) -> Self {
        self.change_intent = Some(intent);
        self
    }

    pub fn with_session_file_service(
        mut self,
        service: Arc<crate::domains::session_files::WorkspaceFileService>,
    ) -> Self {
        self.session_file_service = Some(service);
        self
    }

    pub fn with_session_sandbox_service(
        mut self,
        service: Arc<crate::domains::session_sandbox::SessionSandboxService>,
    ) -> Self {
        self.session_sandbox_service = Some(service);
        self
    }

    pub fn with_session_schedule_service(
        mut self,
        service: Arc<crate::domains::session_schedules::SessionScheduleService>,
    ) -> Self {
        self.session_schedule_service = Some(service);
        self
    }

    pub fn with_notification_service(
        mut self,
        service: Arc<crate::domains::notifications::NotificationService>,
    ) -> Self {
        self.notification_service = Some(service);
        self
    }

    pub fn with_model_service(
        mut self,
        service: Arc<crate::domains::models::ModelService>,
    ) -> Self {
        self.model_service = Some(service);
        self
    }

    pub fn with_provider_service(
        mut self,
        service: Arc<crate::domains::providers::ProviderService>,
    ) -> Self {
        self.provider_service = Some(service);
        self
    }

    pub fn with_model_sync_service(
        mut self,
        service: Arc<crate::domains::models::ModelSyncService>,
    ) -> Self {
        self.model_sync_service = Some(service);
        self
    }

    pub fn with_eval_service(mut self, service: Arc<crate::domains::evals::EvalService>) -> Self {
        self.eval_service = Some(service);
        self
    }

    pub fn with_reporting_service(
        mut self,
        service: Arc<crate::domains::reporting::ReportingService>,
    ) -> Self {
        self.reporting_service = Some(service);
        self
    }

    pub fn with_sqldb_store(
        mut self,
        store: Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore>,
    ) -> Self {
        self.sqldb_store = Some(store);
        self
    }

    pub fn with_workflow_store(
        mut self,
        workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    ) -> Self {
        self.workflow_store = workflow_store;
        self
    }

    pub fn with_runner(mut self, runner: Arc<dyn everruns_core::host::TurnBackend>) -> Self {
        self.runner = Some(runner);
        self
    }

    pub fn with_utility_llm_service(
        mut self,
        service: Arc<dyn everruns_core::UtilityLlmService>,
    ) -> Self {
        self.utility_llm_service = Some(service);
        self
    }

    pub fn with_decisions(mut self, service: Arc<dyn everruns_core::DecisionsService>) -> Self {
        self.decisions = Some(service);
        self
    }

    pub fn with_health_check_service(
        mut self,
        service: Arc<crate::domains::agents::AgentHealthCheckService>,
    ) -> Self {
        self.health_check_service = Some(service);
        self
    }

    pub fn with_fallback_harness_name(mut self, name: Option<String>) -> Self {
        self.fallback_harness_name = name;
        self
    }

    pub fn with_egress_service(mut self, service: Arc<dyn EgressService>) -> Self {
        self.egress_service = Some(service);
        self
    }
    pub fn with_mcp_oauth_checker(
        mut self,
        checker: Option<
            Arc<dyn crate::domains::mcp_servers::connection_check::McpOAuthConnectionChecker>,
        >,
    ) -> Self {
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
}
