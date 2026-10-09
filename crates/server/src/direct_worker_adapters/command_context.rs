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
}
