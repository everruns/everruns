//! The in-process worker's side of `everruns_worker::internal_commands`.
//!
//! A gRPC worker reaches an internal command through `ExecuteCommand`; the
//! in-process worker dispatches the same command here, as the same caller
//! (`Caller::internal`), with the same org feature flags and the same failure
//! mapping (`command_error_to_proto`). The stores built on top are the
//! worker crate's, so the two transports share every line above this one.

use super::DirectWorkerAdapters;
use crate::kernel_imports::Caller;
use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_core::permissions::PermissionResolver;
use everruns_core::session_services::{
    LeasedResourceStore, SessionResourceRegistry, SessionScheduleStore, SessionStorageStore,
};
use everruns_core::session_task::SessionTaskRegistry;
use everruns_internal_protocol::proto;
use everruns_worker::internal_commands::{
    CommandLeasedResourceStore, CommandSessionResourceRegistry, CommandSessionScheduleStore,
    CommandSessionStorageStore, CommandSessionTaskRegistry, InternalCommandTransport,
};
use serde_json::Value;
use std::sync::Arc;

struct DirectInternalCommands {
    db: Arc<crate::storage::StorageBackend>,
    permission_resolver: Arc<dyn PermissionResolver>,
    org_id: i64,
    // What the session-task commands' registry needs: events and wakes (a gRPC
    // worker's `ExecuteCommand` context carries both) and egress for task
    // webhooks, which the in-process worker has always delivered.
    event_service: Arc<crate::services::EventService>,
    runner: Option<Arc<dyn everruns_core::host::TurnBackend>>,
    egress_service: Option<Arc<dyn crate::kernel_imports::EgressService>>,
}

impl DirectWorkerAdapters {
    fn internal_commands(&self, org_id: i64) -> DirectInternalCommands {
        DirectInternalCommands {
            db: self.db.clone(),
            permission_resolver: self.permission_resolver.clone(),
            org_id,
            event_service: self.event_service.clone(),
            runner: self.runner.clone(),
            egress_service: self.egress_service.clone(),
        }
    }

    pub(super) fn command_schedule_store(&self, org_id: i64) -> Arc<dyn SessionScheduleStore> {
        Arc::new(CommandSessionScheduleStore::new(
            self.internal_commands(org_id),
        ))
    }

    pub(super) fn command_session_resource_registry(
        &self,
        org_id: i64,
    ) -> Arc<dyn SessionResourceRegistry> {
        Arc::new(CommandSessionResourceRegistry::new(
            self.internal_commands(org_id),
        ))
    }

    pub(super) fn command_leased_resource_store(
        &self,
        org_id: i64,
    ) -> Arc<dyn LeasedResourceStore> {
        Arc::new(CommandLeasedResourceStore::new(
            self.internal_commands(org_id),
        ))
    }

    pub(super) fn command_session_task_registry(
        &self,
        org_id: i64,
    ) -> Arc<dyn SessionTaskRegistry> {
        Arc::new(CommandSessionTaskRegistry::new(
            self.internal_commands(org_id),
        ))
    }

    /// Values run the internal commands; secrets stay on `secrets` (the
    /// database store) until they move with connections and credentials.
    pub(super) fn command_storage_store(
        &self,
        org_id: i64,
        secrets: Arc<dyn SessionStorageStore>,
    ) -> Arc<dyn SessionStorageStore> {
        Arc::new(CommandSessionStorageStore::new(
            self.internal_commands(org_id),
            secrets,
        ))
    }
}

#[async_trait]
impl InternalCommandTransport for DirectInternalCommands {
    async fn execute_internal_command(
        &self,
        name: &str,
        params: Value,
    ) -> Result<std::result::Result<Value, proto::CommandError>> {
        let feature_flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            self.org_id,
            &crate::records::FeatureFlagPolicy::current(),
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, org_id = self.org_id, "Failed to resolve command feature flags");
            AgentLoopError::store("Failed to resolve organization feature flags")
        })?;
        let mut ctx = crate::domains::common::Ctx::minimal(
            Caller::internal(self.org_id),
            self.db.clone(),
            None,
            self.permission_resolver.clone(),
        )
        .with_feature_flags(feature_flags)
        .with_event_service(self.event_service.clone());
        if let Some(runner) = &self.runner {
            ctx = ctx.with_runner(runner.clone());
        }
        if let Some(egress) = &self.egress_service {
            ctx = ctx.with_egress_service(egress.clone());
        }
        Ok(
            match crate::domains::common::dispatch(name, params, &ctx).await {
                Ok(json) => Ok(serde_json::from_str(&json).map_err(|error| {
                    AgentLoopError::store(format!("Failed to decode command response: {error}"))
                })?),
                Err(error) => Err(crate::worker_link::grpc_service::command_error_to_proto(
                    error,
                )),
            },
        )
    }
}
