//! A `WorkerAdapters` stub for tests that only need the task-worker loop.
//!
//! It lives here rather than inline in `unified_worker`'s test module because
//! the trait has ~60 required methods: three hundred lines of `unimplemented!()`
//! crowd out the tests they exist for, and every method added to the trait grows
//! the file again.

use crate::worker_adapters::WorkerAdapters;
use everruns_provider::error::Result as CoreResult;
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
    ) -> CoreResult<Option<everruns_platform::Agent>> {
        unimplemented!()
    }
    async fn get_harness(
        &self,
        _org_id: i64,
        _harness_id: Uuid,
    ) -> CoreResult<Option<everruns_platform::Harness>> {
        unimplemented!()
    }
    async fn get_session(
        &self,
        _org_id: i64,
        _session_id: Uuid,
    ) -> CoreResult<Option<everruns_core::ExecutionSession>> {
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
    ) -> CoreResult<everruns_core::ExecutionSession> {
        unimplemented!()
    }
    async fn get_message(
        &self,
        _session_id: Uuid,
        _message_id: Uuid,
    ) -> CoreResult<Option<everruns_core::RuntimeMessage>> {
        unimplemented!()
    }
    async fn load_messages(
        &self,
        _session_id: Uuid,
    ) -> CoreResult<Vec<everruns_core::RuntimeMessage>> {
        unimplemented!()
    }
    async fn emit_event(
        &self,
        _request: everruns_core::events::EventRequest,
    ) -> CoreResult<everruns_core::events::Event> {
        unimplemented!()
    }
    async fn get_model_spec(
        &self,
        _org_id: i64,
        _model_id: Uuid,
    ) -> CoreResult<Option<everruns_provider::model_spec::ModelSpec>> {
        unimplemented!()
    }
    async fn get_default_model_spec(
        &self,
        _org_id: i64,
    ) -> CoreResult<Option<everruns_provider::model_spec::ModelSpec>> {
        unimplemented!()
    }
    async fn get_provider_config(
        &self,
        _org_id: i64,
        _provider: &everruns_provider::runtime_provider::ProviderKey,
    ) -> CoreResult<Option<everruns_provider::driver_registry::ProviderConfig>> {
        unimplemented!()
    }
    async fn resolve_image(
        &self,
        _org_id: i64,
        _image_id: Uuid,
    ) -> CoreResult<Option<everruns_core::image_services::ResolvedImage>> {
        unimplemented!()
    }
    async fn resolve_images_batch(
        &self,
        _org_id: i64,
        _image_ids: &[Uuid],
    ) -> CoreResult<HashMap<Uuid, everruns_core::image_services::ResolvedImage>> {
        unimplemented!()
    }
    async fn resolve_files_batch(
        &self,
        _org_id: i64,
        _file_ids: &[Uuid],
    ) -> CoreResult<HashMap<Uuid, everruns_core::file_services::ResolvedFile>> {
        unimplemented!()
    }
    async fn read_file(
        &self,
        _session_id: Uuid,
        _path: &str,
    ) -> CoreResult<Option<everruns_core::session_file::SessionFile>> {
        unimplemented!()
    }
    async fn write_file(
        &self,
        _session_id: Uuid,
        _path: &str,
        _content: &str,
        _encoding: &str,
    ) -> CoreResult<everruns_core::session_file::SessionFile> {
        unimplemented!()
    }
    async fn delete_file(
        &self,
        _session_id: Uuid,
        _path: &str,
        _recursive: bool,
    ) -> CoreResult<bool> {
        unimplemented!()
    }
    async fn list_directory(
        &self,
        _session_id: Uuid,
        _path: &str,
    ) -> CoreResult<Vec<everruns_core::session_file::FileInfo>> {
        unimplemented!()
    }
    async fn stat_file(
        &self,
        _session_id: Uuid,
        _path: &str,
    ) -> CoreResult<Option<everruns_core::session_file::FileStat>> {
        unimplemented!()
    }
    async fn grep_files(
        &self,
        _session_id: Uuid,
        _pattern: &str,
        _path_pattern: Option<&str>,
    ) -> CoreResult<Vec<everruns_core::session_file::GrepMatch>> {
        unimplemented!()
    }
    async fn create_directory(
        &self,
        _session_id: Uuid,
        _path: &str,
    ) -> CoreResult<everruns_core::session_file::FileInfo> {
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
    async fn invoke_scheduled_app_channel(
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
    ) -> CoreResult<Vec<everruns_core::leased_resource::LeasedResource>> {
        unimplemented!()
    }
    async fn mark_leased_resource_released(
        &self,
        _resource_id: everruns_provider::typed_id::LeasedResourceId,
        _expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
    ) -> CoreResult<bool> {
        unimplemented!()
    }
    async fn mark_leased_resource_cleanup_failed(
        &self,
        _resource_id: everruns_provider::typed_id::LeasedResourceId,
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
    ) -> CoreResult<Vec<(everruns_provider::typed_id::SessionId, String)>> {
        unimplemented!()
    }
    async fn prune_terminal_session_tasks(
        &self,
        _ttl: chrono::Duration,
        _limit: i64,
    ) -> CoreResult<usize> {
        unimplemented!()
    }

    fn capability_registry(&self) -> everruns_core::capabilities::CapabilityRegistry {
        unimplemented!()
    }
    fn driver_registry(&self) -> everruns_provider::DriverRegistry {
        unimplemented!()
    }
    fn sqldb_store(
        &self,
        _org_id: i64,
    ) -> std::sync::Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore> {
        unimplemented!()
    }
    fn storage_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn everruns_core::session_services::SessionStorageStore> {
        unimplemented!()
    }
    fn storage_store_unscoped(
        &self,
    ) -> Arc<dyn everruns_core::session_services::SessionStorageStore> {
        unimplemented!()
    }
    fn image_artifact_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn everruns_core::image_services::ImageArtifactStore> {
        unimplemented!()
    }
    fn provider_credential_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn everruns_core::connection_services::ProviderCredentialStore> {
        unimplemented!()
    }
    fn utility_llm_service(&self) -> Option<Arc<dyn everruns_core::UtilityLlmService>> {
        unimplemented!()
    }
    fn egress_service(&self) -> Option<Arc<dyn everruns_core::EgressService>> {
        unimplemented!()
    }
    fn platform_store(
        &self,
        _org_id: i64,
        _session_id: everruns_provider::typed_id::SessionId,
    ) -> Arc<dyn everruns_platform::PlatformStore> {
        unimplemented!()
    }
    fn connection_resolver(
        &self,
    ) -> Arc<dyn everruns_core::connection_services::UserConnectionResolver> {
        unimplemented!()
    }
    fn leased_resource_store(
        &self,
    ) -> Arc<dyn everruns_core::session_services::LeasedResourceStore> {
        unimplemented!()
    }
    fn schedule_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn everruns_core::session_services::SessionScheduleStore> {
        unimplemented!()
    }
    fn reaper_session_task_registry(
        &self,
    ) -> Arc<dyn everruns_core::session_task::SessionTaskRegistry> {
        unimplemented!()
    }
}
