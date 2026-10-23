//! Construct and invoke scoped platform delegation services.
use super::*;
impl DirectPlatformStore {
    pub(super) fn new(
        org_id: i64,
        session_id: SessionId,
        db: Arc<StorageBackend>,
        deps: DirectPlatformStoreDeps,
    ) -> Self {
        let mut capability_service = crate::services::CapabilityService::with_registry(
            db.clone(),
            deps.encryption.clone(),
            deps.capability_registry.clone(),
        );
        if let Some(egress) = &deps.egress_service {
            capability_service = capability_service.with_mcp_egress_service(egress.clone());
        }
        let capability_service = Arc::new(capability_service);
        let session_service = Arc::new(SessionService::with_registry(
            db.clone(),
            deps.capability_registry.clone(),
        ));
        let message_service = deps.runner.as_ref().map(|runner| {
            Arc::new(MessageService::new(
                db.clone(),
                runner.clone(),
                deps.event_service.event_delivery().clone(),
            ))
        });
        Self {
            input_message_id: None,
            org_id,
            session_id,
            db,
            runner: deps.runner,
            capability_service,
            session_service,
            message_service,
            event_service: deps.event_service,
            encryption: deps.encryption,
            workflow_store: deps.workflow_store,
            slack_provisioner: deps.slack_provisioner,
            permission_resolver: deps.permission_resolver,
            connector_registry: deps.connector_registry,
            provider_services: deps.provider_services,
        }
    }

    pub(super) fn base_url_from_env() -> String {
        everruns_core::config::env_string_any(
            &["PUBLIC_APP_URL", "FRONTEND_URL", "APP_URL"],
            "http://localhost:9300",
        )
    }

    pub(super) async fn resolve_caller(&self) -> everruns_contracts::error::Result<Caller> {
        let message = self
            .input_message_id
            .ok_or_else(|| store_error("Management-authorized invocation required"))?;
        let user = self
            .db
            .runtime_invocation_management_user(self.session_id, message)
            .await
            .map_err(|e| store_error(e.to_string()))?
            .ok_or_else(|| store_error("Management-authorized invocation required"))?;
        crate::auth::caller_resolution::caller_for_user(&self.db, self.org_id, user)
            .await
            .map_err(|e| store_error(e.to_string()))
    }

    pub(super) async fn execute_domain_command<T>(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> everruns_contracts::error::Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let ctx = self.command_ctx().await?;
        let json = crate::domains::common::dispatch(name, params, &ctx)
            .await
            .map_err(|e| store_error(format!("Command {name} failed: {e}")))?;
        serde_json::from_str(&json)
            .map_err(|e| store_error(format!("Failed to decode {name} response: {e}")))
    }

    pub(super) async fn execute_runtime_lookup<T>(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> everruns_contracts::error::Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        let ctx = self.command_ctx().await?;
        match super::super::runtime_command_view::dispatch_runtime_view(name, params, &ctx).await {
            Ok(json) => serde_json::from_str(&json)
                .map(Some)
                .map_err(|e| store_error(format!("Failed to decode {name} response: {e}"))),
            Err(crate::domains::common::CommandError {
                kind: crate::domains::common::CommandErrorKind::NotFound(_),
                ..
            }) => Ok(None),
            Err(error) => Err(store_error(format!("Command {name} failed: {error}"))),
        }
    }

    pub(super) async fn invoke_platform_command_surface(
        &self,
        operation: super::super::platform_command_surface::Operation,
        arguments: serde_json::Value,
    ) -> everruns_contracts::error::Result<String> {
        let (base_url, ctx) = (Self::base_url_from_env(), self.command_ctx().await?);
        let context = crate::api::mcp_endpoint::catalog::CatalogContext {
            domain_ctx: crate::domains::change_history::on_platform(ctx, self.session_id).await,
            link_builder: crate::api::common::UrlBuilder::new(&base_url, &base_url),
        };
        super::super::platform_command_surface::invoke(operation, &arguments, context)
            .await
            .map_err(AgentLoopError::tool)
    }

    pub(super) async fn latest_terminal_turn_status(
        &self,
        session_id: SessionId,
    ) -> Result<Option<String>> {
        const TURN_COMPLETED: &str = "turn.completed";
        const TURN_FAILED: &str = "turn.failed";
        const TURN_CANCELLED: &str = "turn.cancelled";
        const TURN_SEALED: &str = "turn.sealed";

        let response: serde_json::Value = self
            .execute_domain_command(
                "list_events",
                serde_json::json!({
                    "session_id": session_id.to_string(),
                    "types": [TURN_COMPLETED, TURN_FAILED, TURN_CANCELLED, TURN_SEALED],
                    "limit": 1,
                    "order_desc": true,
                }),
            )
            .await?;

        let Some(event_type) = response
            .get("data")
            .and_then(|data| data.as_array())
            .and_then(|events| events.first())
            .and_then(|event| event.get("type"))
            .and_then(|event_type| event_type.as_str())
        else {
            return Ok(None);
        };

        let status = match event_type {
            TURN_COMPLETED => Some("completed"),
            TURN_FAILED => Some("failed"),
            TURN_CANCELLED => Some("cancelled"),
            // A sealed turn is terminal but distinct from a failure: surface it
            // as "sealed" so the parent agent can decide what to do next.
            TURN_SEALED => Some("sealed"),
            _ => None,
        };
        Ok(status.map(str::to_string))
    }
}

impl DirectWorkerAdapters {
    /// Create new direct adapters with access to storage and services
    pub fn new(
        db: Arc<StorageBackend>,
        event_service: Arc<EventService>,
        provider_resolver: Arc<ProviderResolverService>,
        mcp_server_service: Arc<McpServerService>,
        capability_registry: CapabilityRegistry,
        driver_registry: DriverRegistry,
        sqldb_store: std::sync::Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore>,
    ) -> Self {
        Self {
            input_message_id: None,
            db,
            event_service,
            budget_service: None,
            provider_resolver,
            mcp_server_service,
            capability_registry,
            connector_registry: everruns_contracts::connector::ConnectorRegistry::new(),
            driver_registry,
            utility_llm_service: None,
            decisions: None,
            egress_service: None,
            sqldb_store,
            storage_store: None,
            connection_resolver: None,
            vector_store: None,
            runner: None,
            encryption: None,
            in_memory_compaction_checkpoint_store: None,
            proactive_compaction_attempts: Arc::default(),
            workflow_store: None,
            slack_provisioner: None,
            channels: None,
            permission_resolver: Arc::new(everruns_core::DefaultPermissionResolver),
            virtual_registry: None,
            org_rate_limiter: None,
            provider_services: None,
            quota: crate::domains::session_files::limits::QuotaLimits::from_env(),
        }
    }
}
