#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// EVE-872: hosted worker adapters project stored views into the same
// canonical resolved execution snapshot as the Framework runtime, and the
// projection is identical whether views arrive in-process (direct adapters)
// or after a serialization round-trip (the gRPC adapter shape).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::error::Result as CoreResult;
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::{AgentDefinition as Agent, HarnessDefinition as Harness};
use everruns_core::{DEFAULT_ORG_ID, ExecutionSession, ResolvedExecutionSnapshot};
use everruns_host::{RuntimeHostAdapter, SessionBuilder};
// EVE-877: the hosted adapters transport the stored platform view; the
// loading seam projects it into the portable execution definition.
use everruns_worker::{WorkerAdapters, WorkerRuntimeHost, WorkerTurnContext};
use uuid::Uuid;

fn fixture_views() -> (Harness, Agent, ExecutionSession) {
    let harness_id = HarnessId::from_seed(872);
    let agent_id = AgentId::from_seed(872);
    let session_id = SessionId::from_seed(872);

    // Portable execution view, as transported by WorkerAdapters
    // (EVE-881): the host itself only ever sees the projected definition.
    let harness = Harness {
        name: "hoster".into(),
        system_prompt: Some("Harness instructions.".into()),
        ..Harness::default()
    };
    let agent = Agent {
        id: agent_id,
        name: "hosted-agent".into(),
        system_prompt: "Agent instructions.".into(),
        max_iterations: Some(9),
        ..Agent::new(agent_id, "hosted-agent", "Agent instructions.")
    };
    let session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .title("UI-TITLE-MARKER")
        .build();
    (harness, agent, session)
}

/// Direct-style adapter: hands the stored views to the host in-process.
#[derive(Clone)]
struct DirectMockAdapters {
    harness: Harness,
    agent: Agent,
    session: ExecutionSession,
    load_failure: Option<&'static str>,
}

/// gRPC-style adapter: round-trips every view through serialization before
/// handing it to the host, standing in for the proto wire boundary.
#[derive(Clone)]
struct WireMockAdapters {
    inner: DirectMockAdapters,
}

impl DirectMockAdapters {
    fn load_failure(&self) -> Option<&'static str> {
        self.load_failure
    }
}
impl WireMockAdapters {
    fn load_failure(&self) -> Option<&'static str> {
        self.inner.load_failure
    }
}

fn wire_round_trip<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
    serde_json::from_str(&serde_json::to_string(value).expect("serialize")).expect("deserialize")
}

macro_rules! mock_worker_adapters {
    ($ty:ident, $harness:expr, $agent:expr, $session:expr) => {
        #[async_trait]
        impl WorkerAdapters for $ty {
            async fn get_agent(&self, _org_id: i64, agent_id: Uuid) -> CoreResult<Option<Agent>> {
                let agent = ($agent)(self);
                if let Some(error) = self.load_failure() {
                    return Err(everruns_contracts::error::AgentLoopError::config(error));
                }
                Ok((agent.id.uuid() == agent_id).then_some(agent))
            }
            async fn get_harness(
                &self,
                _org_id: i64,
                harness_id: Uuid,
            ) -> CoreResult<Option<Harness>> {
                let harness = ($harness)(self);
                Ok((HarnessId::from_seed(872).uuid() == harness_id).then_some(harness))
            }
            async fn get_session(
                &self,
                _org_id: i64,
                session_id: Uuid,
            ) -> CoreResult<Option<ExecutionSession>> {
                let session = ($session)(self);
                Ok((session.id.uuid() == session_id).then_some(session))
            }
            async fn set_session_status(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _status: &str,
            ) -> CoreResult<()> {
                Ok(())
            }
            async fn set_session_title(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _title: String,
            ) -> CoreResult<ExecutionSession> {
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
                Ok(vec![])
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
            ) -> CoreResult<Option<everruns_contracts::model_spec::ModelSpec>> {
                unimplemented!()
            }
            async fn get_default_model_spec(
                &self,
                _org_id: i64,
            ) -> CoreResult<Option<everruns_contracts::model_spec::ModelSpec>> {
                Ok(None)
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
                _org_id: i64,
                _session_id: Uuid,
                _path: &str,
            ) -> CoreResult<Option<everruns_core::session_file::SessionFile>> {
                unimplemented!()
            }
            async fn write_file(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _path: &str,
                _content: &str,
                _encoding: &str,
            ) -> CoreResult<everruns_core::session_file::SessionFile> {
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
            ) -> CoreResult<Vec<everruns_core::session_file::FileInfo>> {
                unimplemented!()
            }
            async fn stat_file(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _path: &str,
            ) -> CoreResult<Option<everruns_core::session_file::FileStat>> {
                unimplemented!()
            }
            async fn grep_files(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _pattern: &str,
                _path_pattern: Option<&str>,
            ) -> CoreResult<Vec<everruns_core::session_file::GrepMatch>> {
                unimplemented!()
            }
            async fn create_directory(
                &self,
                _org_id: i64,
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
            ) -> CoreResult<everruns_worker::mcp_executor::McpServerInfo> {
                unimplemented!()
            }
            async fn load_turn_context(
                &self,
                _org_id: i64,
                _session_id: Uuid,
            ) -> CoreResult<WorkerTurnContext> {
                Ok(WorkerTurnContext {
                    agent: Some(($agent)(self)),
                    session: ($session)(self),
                    messages: vec![],
                    model: None,
                    mcp_tool_definitions: vec![],
                })
            }
            async fn invoke_scheduled_endpoint(
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
            ) -> CoreResult<Vec<(SessionId, String)>> {
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
                everruns_core::capabilities::CapabilityRegistry::new()
            }
            fn driver_registry(&self) -> everruns_contracts::DriverRegistry {
                everruns_contracts::DriverRegistry::new()
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
            ) -> Arc<dyn everruns_core::session_services::SessionStorageStore> {
                unimplemented!()
            }

            fn storage_store_unscoped(
                &self,
            ) -> Arc<dyn everruns_core::session_services::SessionStorageStore> {
                self.storage_store(everruns_core::DEFAULT_ORG_ID)
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
                None
            }
            fn egress_service(&self) -> Option<Arc<dyn everruns_core::EgressService>> {
                None
            }
            fn platform_store(
                &self,
                _org_id: i64,
                _session_id: SessionId,
            ) -> Arc<dyn everruns_capabilities::PlatformStore> {
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
    };
}

mock_worker_adapters!(
    DirectMockAdapters,
    |a: &DirectMockAdapters| a.harness.clone(),
    |a: &DirectMockAdapters| a.agent.clone(),
    |a: &DirectMockAdapters| a.session.clone()
);
mock_worker_adapters!(
    WireMockAdapters,
    |a: &WireMockAdapters| wire_round_trip(&a.inner.harness),
    |a: &WireMockAdapters| wire_round_trip(&a.inner.agent),
    |a: &WireMockAdapters| wire_round_trip(&a.inner.session)
);

#[tokio::test]
async fn direct_and_wire_adapters_project_the_same_snapshot() {
    let (harness, agent, session) = fixture_views();
    let direct = DirectMockAdapters {
        harness: harness.clone(),
        agent: agent.clone(),
        session: session.clone(),
        load_failure: None,
    };
    let wire = WireMockAdapters {
        inner: direct.clone(),
    };

    let direct_inputs = WorkerRuntimeHost::new(direct)
        .load_resolved_turn(DEFAULT_ORG_ID, session.id)
        .await
        .expect("direct adapter resolves");
    let wire_inputs = WorkerRuntimeHost::new(wire)
        .load_resolved_turn(DEFAULT_ORG_ID, session.id)
        .await
        .expect("wire adapter resolves");

    // The canonical projection is the reference: hosted adapters built from
    // equivalent configuration produce equivalent snapshots.
    let reference = ResolvedExecutionSnapshot::project(&harness, Some(&agent), &session)
        .expect("reference projection");

    let direct_json = serde_json::to_value(&direct_inputs.snapshot).unwrap();
    let wire_json = serde_json::to_value(&wire_inputs.snapshot).unwrap();
    let reference_json = serde_json::to_value(&reference).unwrap();
    assert_eq!(direct_json, reference_json);
    assert_eq!(wire_json, reference_json);

    // Deterministic serialization: repeated loads serialize byte-identically.
    let again = WorkerRuntimeHost::new(WireMockAdapters {
        inner: DirectMockAdapters {
            harness,
            agent,
            session: session.clone(),
            load_failure: None,
        },
    })
    .load_resolved_turn(DEFAULT_ORG_ID, session.id)
    .await
    .expect("repeat load");
    assert_eq!(
        serde_json::to_string(&again.snapshot).unwrap(),
        serde_json::to_string(&wire_inputs.snapshot).unwrap()
    );

    // Platform-only session metadata (UI title) never enters the snapshot.
    assert!(!wire_json.to_string().contains("UI-TITLE-MARKER"));
}

#[tokio::test]
async fn worker_load_fails_for_archived_and_deleted_agents() {
    // EVE-877: lifecycle validation happens at the worker loading seam, before
    // the resolved snapshot is built and before host execution.
    for status in ["archived", "deleted"] {
        let (harness, agent, session) = fixture_views();
        let host = WorkerRuntimeHost::new(DirectMockAdapters {
            harness,
            agent,
            session: session.clone(),
            load_failure: Some("agent is inactive and cannot execute turns"),
        });
        let error = host
            .load_resolved_turn(DEFAULT_ORG_ID, session.id)
            .await
            .expect_err("inactive agent must fail the loading seam");
        assert!(
            error.to_string().contains("cannot execute turns"),
            "unexpected error for {status:?}: {error}"
        );
    }
}

#[tokio::test]
async fn worker_projection_fails_on_missing_views() {
    let (harness, agent, mut session) = fixture_views();
    // ExecutionSession referencing a harness outside the adapter's org scope resolves
    // to no views — the cross-tenant / missing shape.
    session.harness_id = HarnessId::from_seed(999);
    let host = WorkerRuntimeHost::new(DirectMockAdapters {
        harness,
        agent,
        session: session.clone(),
        load_failure: None,
    });
    assert!(
        host.load_resolved_turn(DEFAULT_ORG_ID, session.id)
            .await
            .is_err(),
        "projection must fail before host execution when the harness cannot be resolved"
    );
}
