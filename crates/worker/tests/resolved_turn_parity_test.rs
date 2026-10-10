#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// EVE-872: hosted worker adapters project stored views into the same
// canonical resolved execution snapshot as the Framework runtime, and the
// projection is identical whether views arrive in-process (direct adapters)
// or after a serialization round-trip (the gRPC adapter shape).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use everruns_contracts::error::Result as CoreResult;
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use everruns_durable_engine::core::{AgentDefinition as Agent, HarnessDefinition as Harness};
use everruns_durable_engine::core::{DEFAULT_ORG_ID, ExecutionSession, ResolvedExecutionSnapshot};
use everruns_durable_engine::host::{RuntimeHostAdapter, SessionBuilder};
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
    reads: Arc<SourceReads>,
}

#[derive(Default)]
struct SourceReads {
    agents: AtomicU32,
    harnesses: AtomicU32,
    sessions: AtomicU32,
    events: AtomicU32,
}

/// gRPC-style adapter: round-trips every view through serialization before
/// handing it to the host, standing in for the proto wire boundary.
#[derive(Clone)]
struct WireMockAdapters {
    inner: DirectMockAdapters,
}

impl DirectMockAdapters {
    fn source_reads(&self) -> &SourceReads {
        &self.reads
    }
    fn load_failure(&self) -> Option<&'static str> {
        self.load_failure
    }
}
impl WireMockAdapters {
    fn source_reads(&self) -> &SourceReads {
        &self.inner.reads
    }
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
                self.source_reads().agents.fetch_add(1, Ordering::SeqCst);
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
                self.source_reads().harnesses.fetch_add(1, Ordering::SeqCst);
                let harness = ($harness)(self);
                Ok((HarnessId::from_seed(872).uuid() == harness_id).then_some(harness))
            }
            async fn resolve_agent_read(
                &self,
                org_id: i64,
                id: Uuid,
            ) -> CoreResult<(
                CoreResult<Option<Agent>>,
                Option<everruns_durable_engine::core::DependencyBlocker>,
            )> {
                let definition = self.get_agent(org_id, id).await;
                let blocker = match &definition {
                    Ok(None) => Some(everruns_durable_engine::core::DependencyBlocker::AgentDeleted),
                    _ => None,
                };
                Ok((definition, blocker))
            }
            async fn resolve_harness_read(
                &self,
                org_id: i64,
                id: Uuid,
            ) -> CoreResult<(
                CoreResult<Option<Harness>>,
                Option<everruns_durable_engine::core::DependencyBlocker>,
            )> {
                let definition = self.get_harness(org_id, id).await;
                let blocker = match &definition {
                    Ok(None) => Some(everruns_durable_engine::core::DependencyBlocker::HarnessDeleted),
                    _ => None,
                };
                Ok((definition, blocker))
            }
            async fn get_session(
                &self,
                _org_id: i64,
                session_id: Uuid,
            ) -> CoreResult<Option<ExecutionSession>> {
                self.source_reads().sessions.fetch_add(1, Ordering::SeqCst);
                let session = ($session)(self);
                Ok((session.id.uuid() == session_id).then_some(session))
            }
            async fn set_session_status(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _status: &str,
            ) -> CoreResult<()> {
                Err(everruns_contracts::error::AgentLoopError::store(
                    "status write failed",
                ))
            }
            async fn set_session_title(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _title: String,
            ) -> CoreResult<ExecutionSession> {
                Err(everruns_contracts::error::AgentLoopError::store(
                    "title write failed",
                ))
            }
            async fn get_message(
                &self,
                _session_id: Uuid,
                _message_id: Uuid,
            ) -> CoreResult<Option<everruns_durable_engine::core::RuntimeMessage>> {
                unimplemented!()
            }
            async fn load_messages(
                &self,
                _session_id: Uuid,
            ) -> CoreResult<Vec<everruns_durable_engine::core::RuntimeMessage>> {
                Ok(vec![])
            }
            async fn emit_event(
                &self,
                _request: everruns_durable_engine::core::events::EventRequest,
            ) -> CoreResult<everruns_durable_engine::core::events::Event> {
                self.source_reads().events.fetch_add(1, Ordering::SeqCst);
                Err(everruns_contracts::error::AgentLoopError::store(
                    "event emission failed",
                ))
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
            ) -> CoreResult<Option<everruns_durable_engine::core::image_services::ResolvedImage>> {
                unimplemented!()
            }
            async fn resolve_images_batch(
                &self,
                _org_id: i64,
                _image_ids: &[Uuid],
            ) -> CoreResult<HashMap<Uuid, everruns_durable_engine::core::image_services::ResolvedImage>> {
                unimplemented!()
            }
            async fn resolve_files_batch(
                &self,
                _org_id: i64,
                _file_ids: &[Uuid],
            ) -> CoreResult<HashMap<Uuid, everruns_durable_engine::core::file_services::ResolvedFile>> {
                unimplemented!()
            }
            async fn read_file(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _path: &str,
            ) -> CoreResult<Option<everruns_durable_engine::core::session_file::SessionFile>> {
                unimplemented!()
            }
            async fn write_file(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _path: &str,
                _content: &str,
                _encoding: &str,
            ) -> CoreResult<everruns_durable_engine::core::session_file::SessionFile> {
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
            ) -> CoreResult<Vec<everruns_durable_engine::core::session_file::FileInfo>> {
                unimplemented!()
            }
            async fn stat_file(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _path: &str,
            ) -> CoreResult<Option<everruns_durable_engine::core::session_file::FileStat>> {
                unimplemented!()
            }
            async fn grep_files(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _pattern: &str,
                _path_pattern: Option<&str>,
            ) -> CoreResult<Vec<everruns_durable_engine::core::session_file::GrepMatch>> {
                unimplemented!()
            }
            async fn create_directory(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _path: &str,
            ) -> CoreResult<everruns_durable_engine::core::session_file::FileInfo> {
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
                if let Some(error) = self.load_failure() {
                    return Err(everruns_contracts::error::AgentLoopError::config(error));
                }
                Ok(WorkerTurnContext {
                    agent: Some(($agent)(self)),
                    session: ($session)(self),
                    messages: vec![],
                    model: None,
                    mcp_tool_definitions: vec![],
                })
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
            ) -> CoreResult<Vec<(i64, everruns_durable_engine::core::leased_resource::LeasedResource)>> {
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
            ) -> CoreResult<Vec<(i64, SessionId, String)>> {
                unimplemented!()
            }
            async fn prune_terminal_session_tasks(
                &self,
                _ttl: chrono::Duration,
                _limit: i64,
            ) -> CoreResult<usize> {
                unimplemented!()
            }
            fn capability_registry(&self) -> everruns_durable_engine::core::capabilities::CapabilityRegistry {
                everruns_durable_engine::core::capabilities::CapabilityRegistry::new()
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
            ) -> Arc<dyn everruns_durable_engine::core::session_services::SessionStorageStore> {
                unimplemented!()
            }

            fn image_artifact_store(
                &self,
                _org_id: i64,
            ) -> Arc<dyn everruns_durable_engine::core::image_services::ImageArtifactStore> {
                unimplemented!()
            }
            fn provider_credential_store(
                &self,
                _org_id: i64,
            ) -> Arc<dyn everruns_durable_engine::core::connection_services::ProviderCredentialStore> {
                unimplemented!()
            }
            fn utility_llm_service(&self) -> Option<Arc<dyn everruns_durable_engine::core::UtilityLlmService>> {
                None
            }
            fn egress_service(&self) -> Option<Arc<dyn everruns_durable_engine::core::EgressService>> {
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
                _org_id: i64,
            ) -> Arc<dyn everruns_durable_engine::core::connection_services::UserConnectionResolver> {
                unimplemented!()
            }
            fn leased_resource_store(
                &self,
                _org_id: i64,
            ) -> Arc<dyn everruns_durable_engine::core::session_services::LeasedResourceStore> {
                unimplemented!()
            }
            fn schedule_store(
                &self,
                _org_id: i64,
            ) -> Arc<dyn everruns_durable_engine::core::session_services::SessionScheduleStore> {
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
        reads: Arc::default(),
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
            reads: Arc::default(),
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
async fn worker_load_propagates_control_plane_projection_errors() {
    let (harness, agent, session) = fixture_views();
    let host = WorkerRuntimeHost::new(DirectMockAdapters {
        harness,
        agent,
        session: session.clone(),
        load_failure: Some("control-plane projection refused execution"),
        reads: Arc::default(),
    });
    let error = host
        .load_resolved_turn(DEFAULT_ORG_ID, session.id)
        .await
        .expect_err("projection refusal must stop snapshot loading");
    assert!(
        error
            .to_string()
            .contains("control-plane projection refused execution")
    );
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
        reads: Arc::default(),
    });
    assert!(
        host.load_resolved_turn(DEFAULT_ORG_ID, session.id)
            .await
            .is_err(),
        "projection must fail before host execution when the harness cannot be resolved"
    );
}

fn phase_fixture() -> DirectMockAdapters {
    let (harness, agent, session) = fixture_views();
    DirectMockAdapters {
        harness,
        agent,
        session,
        load_failure: None,
        reads: Arc::default(),
    }
}

#[tokio::test]
async fn worker_host_shares_setup_definition_and_blocker_reads() {
    let adapters = phase_fixture();
    let counts = adapters.reads.clone();
    let agent_id = adapters.agent.id;
    let harness_id = adapters.session.harness_id;
    let host = WorkerRuntimeHost::new(adapters.clone());
    let agents = host.agent_store(DEFAULT_ORG_ID);
    let harnesses = host.harness_store(DEFAULT_ORG_ID);
    assert!(agents.get_agent_blocker(agent_id).await.unwrap().is_none());
    assert_eq!(
        agents.get_agent(agent_id).await.unwrap().unwrap().id,
        agent_id
    );
    assert!(
        harnesses
            .get_harness_blocker(harness_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        harnesses
            .get_harness(harness_id)
            .await
            .unwrap()
            .unwrap()
            .name,
        adapters.harness.name
    );
    assert_eq!(counts.agents.load(Ordering::SeqCst), 1);
    assert_eq!(counts.harnesses.load(Ordering::SeqCst), 1);
    // Each activity owns its own phase, even when adapters share a backend.
    let next = WorkerRuntimeHost::new(adapters);
    next.agent_store(DEFAULT_ORG_ID)
        .get_agent(agent_id)
        .await
        .unwrap();
    assert_eq!(counts.agents.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn batch_seed_is_pinned_and_failed_session_writes_still_invalidate() {
    let adapters = phase_fixture();
    let counts = adapters.reads.clone();
    let session_id = adapters.session.id;
    let agent_id = adapters.agent.id;
    let host = WorkerRuntimeHost::new(adapters.clone());
    let resolved = host
        .load_resolved_turn(DEFAULT_ORG_ID, session_id)
        .await
        .unwrap();
    let reference = ResolvedExecutionSnapshot::project(
        &adapters.harness,
        Some(&adapters.agent),
        &adapters.session,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(resolved.snapshot).unwrap(),
        serde_json::to_value(reference).unwrap()
    );
    host.agent_store(DEFAULT_ORG_ID)
        .get_agent_blocker(agent_id)
        .await
        .unwrap();
    let agent = host
        .agent_store(DEFAULT_ORG_ID)
        .get_agent(agent_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(agent.system_prompt, adapters.agent.system_prompt);
    assert_eq!(
        counts.agents.load(Ordering::SeqCst),
        0,
        "batch projection is the pinned agent seed"
    );
    host.harness_store(DEFAULT_ORG_ID)
        .get_harness_blocker(adapters.session.harness_id)
        .await
        .unwrap();
    assert_eq!(counts.harnesses.load(Ordering::SeqCst), 1);
    let sessions = host.session_store(DEFAULT_ORG_ID);
    sessions.get_session(session_id).await.unwrap();
    assert_eq!(counts.sessions.load(Ordering::SeqCst), 0);
    assert!(
        host.set_session_status(
            DEFAULT_ORG_ID,
            session_id,
            everruns_durable_engine::core::SessionExecutionState::Active
        )
        .await
        .is_err()
    );
    sessions.get_session(session_id).await.unwrap();
    assert_eq!(counts.sessions.load(Ordering::SeqCst), 1);
    assert!(
        host.session_mutator(DEFAULT_ORG_ID)
            .update_session_title(session_id, "new title".into())
            .await
            .is_err()
    );
    sessions.get_session(session_id).await.unwrap();
    assert_eq!(counts.sessions.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_queued_phase_start_failure_still_ends_setup_memoizing() {
    let adapters = phase_fixture();
    let counts = adapters.reads.clone();
    let id = adapters.agent.id;
    let request = everruns_durable_engine::core::events::EventRequest::new(
        adapters.session.id,
        Default::default(),
        everruns_durable_engine::core::events::ReasonStartedData {
            harness_id: adapters.session.harness_id,
            agent_id: Some(id),
            metadata: None,
        },
    );
    let host = WorkerRuntimeHost::new(adapters);
    let store = host.agent_store(DEFAULT_ORG_ID);
    store.get_agent(id).await.unwrap();
    // Setup ends when the event is queued, before its background store fails.
    assert!(host.event_emitter().emit(request).await.is_ok());
    store.get_agent(id).await.unwrap();
    store.get_agent(id).await.unwrap();
    assert_eq!(
        counts.agents.load(Ordering::SeqCst),
        3,
        "queued emission cannot retain a setup cache"
    );
    host.flush_events().await;
    assert_eq!(counts.events.load(Ordering::SeqCst), 1);
    store.get_agent(id).await.unwrap();
    assert_eq!(
        counts.agents.load(Ordering::SeqCst),
        4,
        "failed background emission cannot restore a setup cache"
    );
}
