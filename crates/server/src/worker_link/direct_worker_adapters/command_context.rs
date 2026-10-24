use super::*;

impl DirectWorkerAdapters {
    /// Share the `/v1/providers` service instances with platform-tool commands,
    /// so a provider write invalidates the resolver cache the runtime reads.
    pub fn with_provider_services(
        mut self,
        services: crate::domains::providers::ProviderServices,
    ) -> Self {
        self.provider_services = Some(services);
        self
    }

    pub fn with_slack_provisioner(
        mut self,
        provisioner: Option<Arc<dyn SlackAppProvisioner>>,
    ) -> Self {
        self.slack_provisioner = provisioner;
        self
    }

    /// The channel host targeted schedule triggers run on.
    pub fn with_channels(
        mut self,
        channels: Option<Arc<crate::channels::host::ServerChannels>>,
    ) -> Self {
        self.channels = channels;
        self
    }

    pub(super) fn platform_store_deps(&self) -> DirectPlatformStoreDeps {
        // Standalone adapters (tests, embedders) build their own over the
        // shared resolver; `app_builder` always injects the routes' instances.
        let provider_services = self.provider_services.clone().unwrap_or_else(|| {
            crate::domains::providers::ProviderServices::new(
                self.db.clone(),
                self.encryption.clone(),
                Arc::new(self.driver_registry.clone()),
                Some(self.provider_resolver.clone()),
            )
        });
        DirectPlatformStoreDeps {
            event_service: self.event_service.clone(),
            runner: self.runner.clone(),
            capability_registry: self.capability_registry.clone(),
            connector_registry: self.connector_registry.clone(),
            encryption: self.encryption.clone(),
            workflow_store: self.workflow_store.clone(),
            slack_provisioner: self.slack_provisioner.clone(),
            permission_resolver: self.permission_resolver.clone(),
            egress_service: self.egress_service.clone(),
            provider_services,
        }
    }
}

impl DirectPlatformStore {
    pub(super) async fn command_ctx(
        &self,
    ) -> everruns_contracts::error::Result<crate::domains::common::Ctx> {
        let caller = self.resolve_caller().await?;
        let feature_flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            self.org_id,
            &crate::records::FeatureFlagPolicy::current(),
        )
        .await
        .map_err(|error| {
            store_error(format!("Failed to resolve platform feature flags: {error}"))
        })?;
        let mut ctx = crate::domains::common::Ctx::new(
            caller,
            self.db.clone(),
            self.capability_service.clone(),
            self.encryption.clone(),
            self.permission_resolver.clone(),
        )
        .with_slack_provisioner(self.slack_provisioner.clone())
        .with_feature_flags(feature_flags)
        .with_connector_registry(self.connector_registry.clone())
        .with_workflow_store(self.workflow_store.clone())
        .with_session_service(self.session_service.clone())
        // wait_for_idle probes terminal turn events through list_events; without
        // the event service every subagent wait fails in the in-process adapter.
        .with_event_service(self.event_service.clone())
        // Without these `sync_provider_models` answered 503 and provider
        // create/update skipped model discovery (EVE-1234).
        .with_provider_services(&self.provider_services);

        if let Some(message_service) = &self.message_service {
            ctx = ctx.with_message_service(message_service.clone());
        }
        if let Some(runner) = &self.runner {
            ctx = ctx.with_runner(runner.clone());
        }

        Ok(ctx)
    }

    /// Run a runtime-view mutation a built-in tool makes from this session.
    /// It records like the gRPC worker path: through the session, with the
    /// tool's own reason.
    pub(super) async fn execute_runtime_command<T>(
        &self,
        name: &str,
        params: serde_json::Value,
        reason: &str,
    ) -> everruns_contracts::error::Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        if let Some(message) = self.input_message_id
            && let Some(output) = crate::worker_link::consumer_delegation::dispatch(
                &self.db,
                &self.session_service,
                self.encryption.as_ref(),
                self.message_service.as_deref(),
                self.org_id,
                self.session_id,
                message,
                name,
                &params,
            )
            .await
            .map_err(|e| store_error(e.to_string()))?
        {
            return serde_json::from_str(&output).map_err(|e| store_error(e.to_string()));
        }
        let intent = crate::domains::change_history::agent_session_intent(
            &self.db,
            self.org_id,
            self.session_id,
            crate::domains::change_history::ChangeSurface::Worker,
        )
        .await
        .with_reason(Some(reason.to_string()));
        let ctx = self.command_ctx().await?.with_change_intent(intent);
        let json =
            crate::worker_link::runtime_command_view::dispatch_runtime_view(name, params, &ctx)
                .await
                .map_err(|e| store_error(format!("Command {name} failed: {e}")))?;
        serde_json::from_str(&json)
            .map_err(|e| store_error(format!("Failed to decode {name} response: {e}")))
    }
}
