//! A `WorkerAdapters` stub for tests that only need the task-worker loop.
//!
//! It lives here rather than inline in `unified_worker`'s test module because
//! the trait has ~60 required methods: three hundred lines of `unimplemented!()`
//! crowd out the tests they exist for, and every method added to the trait grows
//! the file again.

use crate::worker_adapters::WorkerAdapters;
use everruns_contracts::error::Result as CoreResult;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct NoopAdapters;

#[async_trait::async_trait]
impl WorkerAdapters for NoopAdapters {
    async fn get_agent(
        &self,
        _org_id: i64,
        _agent_id: Uuid,
    ) -> CoreResult<Option<crate::core::AgentDefinition>> {
        unimplemented!()
    }
    async fn get_harness(
        &self,
        _org_id: i64,
        _harness_id: Uuid,
    ) -> CoreResult<Option<crate::core::HarnessDefinition>> {
        unimplemented!()
    }
    async fn get_session(
        &self,
        _org_id: i64,
        _session_id: Uuid,
    ) -> CoreResult<Option<crate::core::ExecutionSession>> {
        unimplemented!()
    }
    async fn set_session_status(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _status: &str,
    ) -> CoreResult<()> {
        unimplemented!()
    }
    async fn set_session_title(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _title: String,
    ) -> CoreResult<crate::core::ExecutionSession> {
        unimplemented!()
    }
    async fn get_message(
        &self,
        _session_id: Uuid,
        _message_id: Uuid,
    ) -> CoreResult<Option<crate::core::RuntimeMessage>> {
        unimplemented!()
    }
    async fn load_messages(
        &self,
        _session_id: Uuid,
    ) -> CoreResult<Vec<crate::core::RuntimeMessage>> {
        unimplemented!()
    }
    async fn emit_event(
        &self,
        _request: crate::core::events::EventRequest,
    ) -> CoreResult<crate::core::events::Event> {
        unimplemented!()
    }
    async fn emit_events_fenced(
        &self,
        _requests: Vec<crate::core::events::EventRequest>,
        _fence: &crate::actor_lease_store::ActorFence,
    ) -> CoreResult<Option<crate::core::events::Event>> {
        unimplemented!()
    }
    async fn get_model_spec(
        &self,
        _org_id: i64,
        _model_id: Uuid,
    ) -> CoreResult<Option<everruns_contracts::model_spec::ModelSpec>> {
        unimplemented!()
    }
    async fn get_default_model_spec(
        &self,
        _org_id: i64,
    ) -> CoreResult<Option<everruns_contracts::model_spec::ModelSpec>> {
        unimplemented!()
    }
    async fn get_provider_config(
        &self,
        _org_id: i64,
        _provider: &everruns_contracts::runtime_provider::ProviderKey,
    ) -> CoreResult<Option<everruns_contracts::driver_registry::ProviderConfig>> {
        unimplemented!()
    }
    async fn resolve_image(
        &self,
        _org_id: i64,
        _image_id: Uuid,
    ) -> CoreResult<Option<crate::core::image_services::ResolvedImage>> {
        unimplemented!()
    }
    async fn resolve_images_batch(
        &self,
        _org_id: i64,
        _image_ids: &[Uuid],
    ) -> CoreResult<HashMap<Uuid, crate::core::image_services::ResolvedImage>> {
        unimplemented!()
    }
    async fn resolve_files_batch(
        &self,
        _org_id: i64,
        _file_ids: &[Uuid],
    ) -> CoreResult<HashMap<Uuid, crate::core::file_services::ResolvedFile>> {
        unimplemented!()
    }
    async fn read_file(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _path: &str,
    ) -> CoreResult<Option<crate::core::session_file::SessionFile>> {
        unimplemented!()
    }
    async fn write_file(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _path: &str,
        _content: &str,
        _encoding: &str,
    ) -> CoreResult<crate::core::session_file::SessionFile> {
        unimplemented!()
    }
    async fn delete_file(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _path: &str,
        _recursive: bool,
    ) -> CoreResult<bool> {
        unimplemented!()
    }
    async fn list_directory(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _path: &str,
    ) -> CoreResult<Vec<crate::core::session_file::FileInfo>> {
        unimplemented!()
    }
    async fn stat_file(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _path: &str,
    ) -> CoreResult<Option<crate::core::session_file::FileStat>> {
        unimplemented!()
    }
    async fn grep_files(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _pattern: &str,
        _path_pattern: Option<&str>,
    ) -> CoreResult<Vec<crate::core::session_file::GrepMatch>> {
        unimplemented!()
    }
    async fn create_directory(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _path: &str,
    ) -> CoreResult<crate::core::session_file::FileInfo> {
        unimplemented!()
    }
    async fn get_mcp_server_by_prefix(
        &self,
        _org_id: i64,
        _session_id: Option<Uuid>,
        _server_prefix: &str,
    ) -> CoreResult<crate::mcp_executor::McpServerInfo> {
        unimplemented!()
    }
    async fn load_turn_context(
        &self,
        _org_id: i64,
        _session_id: Uuid,
    ) -> CoreResult<crate::worker_adapters::TurnContext> {
        unimplemented!()
    }
    async fn invoke_scheduled_channel(
        &self,
        _org_id: i64,
        _app_id: &str,
        _channel_id: &str,
    ) -> CoreResult<serde_json::Value> {
        unimplemented!()
    }
    async fn invoke_agent_trigger(
        &self,
        _org_id: i64,
        _agent_id: &str,
        _trigger_id: &str,
    ) -> CoreResult<serde_json::Value> {
        unimplemented!()
    }
    async fn claim_due_leased_resources(
        &self,
        _limit: u32,
        _stale_after_seconds: u32,
    ) -> CoreResult<Vec<(i64, crate::core::leased_resource::LeasedResource)>> {
        unimplemented!()
    }
    async fn mark_leased_resource_released(
        &self,
        _resource_id: everruns_contracts::typed_id::LeasedResourceId,
        _expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
    ) -> CoreResult<bool> {
        unimplemented!()
    }
    async fn mark_leased_resource_cleanup_failed(
        &self,
        _resource_id: everruns_contracts::typed_id::LeasedResourceId,
        _expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
        _retry_after_seconds: u32,
        _error: &str,
    ) -> CoreResult<bool> {
        unimplemented!()
    }
    async fn list_orphaned_session_task_ids(
        &self,
        _stale_after: chrono::Duration,
        _limit: i64,
    ) -> CoreResult<Vec<(i64, everruns_contracts::typed_id::SessionId, String)>> {
        unimplemented!()
    }
    async fn prune_terminal_session_tasks(
        &self,
        _ttl: chrono::Duration,
        _limit: i64,
    ) -> CoreResult<usize> {
        unimplemented!()
    }

    /// No actors are ever runnable: the worker's actor loop idles.
    fn actor_lease_store(&self) -> std::sync::Arc<dyn crate::actor_lease_store::ActorLeaseStore> {
        std::sync::Arc::new(IdleActorLeaseStore)
    }

    fn capability_registry(&self) -> crate::core::capabilities::CapabilityRegistry {
        unimplemented!()
    }
    fn driver_registry(&self) -> everruns_contracts::DriverRegistry {
        unimplemented!()
    }
    fn sqldb_store(
        &self,
        _org_id: i64,
    ) -> std::sync::Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore> {
        unimplemented!()
    }
    fn storage_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn crate::core::session_services::SessionStorageStore> {
        unimplemented!()
    }
    fn image_artifact_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn crate::core::image_services::ImageArtifactStore> {
        unimplemented!()
    }
    fn provider_credential_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn crate::core::connection_services::ProviderCredentialStore> {
        unimplemented!()
    }
    fn utility_llm_service(&self) -> Option<Arc<dyn crate::core::UtilityLlmService>> {
        unimplemented!()
    }
    fn egress_service(&self) -> Option<Arc<dyn crate::core::EgressService>> {
        unimplemented!()
    }
    fn platform_store(
        &self,
        _org_id: i64,
        _session_id: everruns_contracts::typed_id::SessionId,
    ) -> Arc<dyn everruns_capabilities::PlatformStore> {
        unimplemented!()
    }
    fn connection_resolver(
        &self,
        _org_id: i64,
    ) -> Arc<dyn crate::core::connection_services::UserConnectionResolver> {
        unimplemented!()
    }
    fn leased_resource_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn crate::core::session_services::LeasedResourceStore> {
        unimplemented!()
    }
    fn schedule_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn crate::core::session_services::SessionScheduleStore> {
        unimplemented!()
    }
}

/// An actor index with nothing runnable.
struct IdleActorLeaseStore;

#[async_trait::async_trait]
impl crate::actor_lease_store::ActorLeaseStore for IdleActorLeaseStore {
    async fn lease_actors(
        &self,
        _holder: &str,
        _limit: u32,
        _ttl: std::time::Duration,
        _max_leased_per_org: Option<u32>,
    ) -> CoreResult<Vec<crate::actor_lease_store::LeasedActor>> {
        Ok(Vec::new())
    }

    async fn renew_actor_lease(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
        _holder: &str,
        _fence: u64,
        _ttl: std::time::Duration,
        _consumed_seq: i64,
    ) -> CoreResult<Option<crate::actor_lease_store::ActorRenewal>> {
        Ok(None)
    }

    async fn release_actor(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
        _holder: &str,
        _fence: u64,
        _consumed_inbox_seq: i64,
    ) -> CoreResult<crate::actor_lease_store::ActorRelease> {
        Ok(crate::actor_lease_store::ActorRelease::LeaseLost)
    }

    async fn read_actor_journal(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
        _after_sequence: Option<i32>,
        _limit: u32,
    ) -> CoreResult<crate::actor_lease_store::ActorJournal> {
        Ok(crate::actor_lease_store::ActorJournal {
            events: Vec::new(),
            has_more: false,
            head_sequence: None,
        })
    }
}
