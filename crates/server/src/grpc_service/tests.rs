use super::*;
use tonic::service::Interceptor;

// Env-var-mutating tests must not run in parallel.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

async fn test_worker_service() -> WorkerServiceImpl {
    test_worker_service_with_runner(None).await
}

async fn test_worker_service_with_runner(
    runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
) -> WorkerServiceImpl {
    let db = Arc::new(StorageBackend::in_memory());
    let grade = everruns_core::DeploymentGrade::Dev;
    let platform_definition = crate::oss_platform_definition_for_grade(grade);
    let encryption = Some(Arc::new(
        EncryptionService::new("kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=", &[])
            .expect("valid test encryption key"),
    ));

    crate::seed::seed_all(&db, grade, &crate::seed::SeedAuthContext::default())
        .await
        .expect("seed test data");

    let event_service =
        EventService::with_listeners(db.clone(), crate::EventDelivery::in_memory(), vec![]);

    WorkerServiceImpl::new(event_service, db, encryption, runner, platform_definition)
}

#[derive(Clone)]
struct CompletingTestRunner {
    db: Arc<StorageBackend>,
    event_service: EventService,
}

#[async_trait::async_trait]
impl everruns_worker::AgentRunner for CompletingTestRunner {
    async fn start_run(
        &self,
        org_id: i64,
        session_id: everruns_core::SessionId,
        _harness_id: everruns_core::HarnessId,
        _agent_id: Option<everruns_core::typed_id::AgentId>,
        _input_message_id: everruns_core::MessageId,
        _request_id: Option<String>,
    ) -> anyhow::Result<()> {
        self.event_service
            .emit(everruns_core::EventRequest::new(
                session_id,
                everruns_core::events::EventContext::empty(),
                everruns_core::events::OutputMessageCompletedData::new(
                    everruns_core::Message::assistant("Child completed through gRPC"),
                ),
            ))
            .await?;
        self.db
            .update_session(
                org_id,
                session_id,
                crate::storage::models::UpdateSession {
                    status: Some("idle".to_string()),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    async fn resume_after_tool_results(
        &self,
        _session_id: everruns_core::SessionId,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn cancel_run(&self, _run_id: everruns_core::SessionId) -> anyhow::Result<()> {
        Ok(())
    }

    async fn is_running(&self, _run_id: everruns_core::SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

async fn test_worker_service_with_completing_runner() -> WorkerServiceImpl {
    let db = Arc::new(StorageBackend::in_memory());
    let grade = everruns_core::DeploymentGrade::Dev;
    let platform_definition = crate::oss_platform_definition_for_grade(grade);
    let encryption = Some(Arc::new(
        EncryptionService::new("kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=", &[])
            .expect("valid test encryption key"),
    ));

    crate::seed::seed_all(&db, grade, &crate::seed::SeedAuthContext::default())
        .await
        .expect("seed test data");

    let event_service =
        EventService::with_listeners(db.clone(), crate::EventDelivery::in_memory(), vec![]);
    let runner = Arc::new(CompletingTestRunner {
        db: db.clone(),
        event_service: event_service.clone(),
    });

    WorkerServiceImpl::new(
        event_service,
        db,
        encryption,
        Some(runner),
        platform_definition,
    )
}

struct AllowingConnectionResolver;

#[async_trait::async_trait]
impl everruns_core::traits::UserConnectionResolver for AllowingConnectionResolver {
    async fn get_connection_token(
        &self,
        _session_id: everruns_core::SessionId,
        _provider: &str,
    ) -> everruns_core::error::Result<Option<String>> {
        Ok(Some("test-token".to_string()))
    }

    async fn get_connection_user(
        &self,
        _session_id: everruns_core::SessionId,
        _provider: &str,
    ) -> everruns_core::error::Result<Option<uuid::Uuid>> {
        Ok(None)
    }

    async fn get_connection_token_for_user(
        &self,
        _user_id: uuid::Uuid,
        _provider: &str,
    ) -> everruns_core::error::Result<Option<String>> {
        Ok(Some("test-token".to_string()))
    }
}

#[test]
fn test_image_info_row_to_proto_uses_raw_uuid_transport_value() {
    let image_id = everruns_core::ImageId::new();
    let proto = WorkerServiceImpl::image_info_row_to_proto(crate::storage::models::ImageInfoRow {
        id: image_id,
        org_id: everruns_core::DEFAULT_ORG_ID,
        filename: "generated-image.png".to_string(),
        content_type: "image/png".to_string(),
        size_bytes: 128,
        metadata: serde_json::json!({ "provider": "openai" }),
        created_at: chrono::Utc::now(),
    });

    let serialized_id = proto
        .id
        .expect("stored image info should include an id")
        .value;

    assert_eq!(serialized_id, image_id.uuid().to_string());
    assert_ne!(serialized_id, image_id.to_string());
}

#[test]
fn test_interceptor_allows_when_no_token_configured() {
    let mut interceptor = GrpcAuthInterceptor::new(None);
    let request = Request::new(());
    assert!(interceptor.call(request).is_ok());
}

#[test]
fn test_interceptor_allows_valid_bearer_token() {
    let mut interceptor = GrpcAuthInterceptor::new(Some("secret123".to_string()));
    let mut request = Request::new(());
    request
        .metadata_mut()
        .insert("authorization", "Bearer secret123".parse().unwrap());
    assert!(interceptor.call(request).is_ok());
}

#[test]
fn test_interceptor_rejects_missing_token() {
    let mut interceptor = GrpcAuthInterceptor::new(Some("secret123".to_string()));
    let request = Request::new(());
    let err = interceptor.call(request).unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    assert!(err.message().contains("Missing"));
}

#[test]
fn test_interceptor_rejects_wrong_token() {
    let mut interceptor = GrpcAuthInterceptor::new(Some("secret123".to_string()));
    let mut request = Request::new(());
    request
        .metadata_mut()
        .insert("authorization", "Bearer wrong_token".parse().unwrap());
    let err = interceptor.call(request).unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    assert!(err.message().contains("Invalid"));
}

#[test]
fn test_interceptor_rejects_non_bearer_scheme() {
    let mut interceptor = GrpcAuthInterceptor::new(Some("secret123".to_string()));
    let mut request = Request::new(());
    request
        .metadata_mut()
        .insert("authorization", "Basic secret123".parse().unwrap());
    let err = interceptor.call(request).unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn test_list_commands_includes_platform_management_commands() {
    let service = test_worker_service().await;

    let response = service
        .list_commands(Request::new(ListCommandsRequest {}))
        .await
        .expect("list_commands should succeed")
        .into_inner();

    assert!(
        response
            .commands
            .iter()
            .any(|command| command.name == "list_harnesses" && command.api_version == "v1")
    );
    assert!(
        response
            .commands
            .iter()
            .any(|command| command.name == "create_app" && command.api_version == "v1")
    );
}

#[tokio::test]
async fn test_execute_command_lists_seeded_harnesses() {
    let service = test_worker_service().await;

    let response = service
        .execute_command(Request::new(ExecuteCommandRequest {
            name: "list_harnesses".to_string(),
            api_version: "v1".to_string(),
            params_json: br#"{}"#.to_vec(),
            org_id: everruns_core::DEFAULT_ORG_ID,
            user_id: None,
            idempotency_key: None,
            metadata: Default::default(),
        }))
        .await
        .expect("execute_command should succeed")
        .into_inner();

    let proto::execute_command_response::Result::OkJson(ok_json) =
        response.result.expect("command result should be present")
    else {
        panic!("expected OkJson response");
    };

    let harnesses: serde_json::Value =
        serde_json::from_slice(&ok_json).expect("response should be valid JSON");
    let names: Vec<&str> = harnesses
        .as_array()
        .expect("list_harnesses should return an array")
        .iter()
        .filter_map(|h| h.get("name").and_then(|name| name.as_str()))
        .collect();

    assert!(names.contains(&"platform-chat"));
}

#[tokio::test]
async fn test_execute_command_unknown_command_returns_bad_request_kind() {
    let service = test_worker_service().await;

    let response = service
        .execute_command(Request::new(ExecuteCommandRequest {
            name: "definitely_not_a_command".to_string(),
            api_version: "v1".to_string(),
            params_json: br#"{}"#.to_vec(),
            org_id: everruns_core::DEFAULT_ORG_ID,
            user_id: None,
            idempotency_key: None,
            metadata: Default::default(),
        }))
        .await
        .expect("execute_command should return a structured command error")
        .into_inner();

    let proto::execute_command_response::Result::Error(error) =
        response.result.expect("command result should be present")
    else {
        panic!("expected Error response");
    };

    assert_eq!(error.kind, 1);
    assert!(error.message.contains("Unknown command"));
}

async fn create_grpc_test_session(service: &WorkerServiceImpl) -> proto::Session {
    let harness = service
        .platform_list_harnesses(Request::new(PlatformListHarnessesRequest {
            org_id: everruns_core::DEFAULT_ORG_ID,
        }))
        .await
        .expect("list harnesses should succeed")
        .into_inner()
        .harnesses
        .into_iter()
        .next()
        .expect("seeded harness");

    service
        .platform_create_session(Request::new(PlatformCreateSessionRequest {
            org_id: everruns_core::DEFAULT_ORG_ID,
            harness_id: harness.id,
            agent_id: None,
            title: None,
            locale: None,
            blueprint_id: None,
            blueprint_config_json: None,
        }))
        .await
        .expect("create session should succeed")
        .into_inner()
        .session
        .expect("session response")
}

fn proto_session_id(session: &proto::Session) -> everruns_core::SessionId {
    let id = session.id.as_ref().expect("session id");
    everruns_core::SessionId::from_uuid(id.value.parse().expect("uuid session id"))
}

async fn start_grpc_test_server(
    service: WorkerServiceImpl,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind grpc listener");
    let addr = listener.local_addr().expect("grpc listener address");
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let (incoming_tx, incoming_rx) = tokio::sync::mpsc::channel(8);
    let server = tokio::spawn(async move {
        let accept_task = tokio::spawn(async move {
            let mut shutdown_rx = shutdown_rx;
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        match accepted {
                            Ok((stream, _)) => {
                                if incoming_tx.send(Ok(stream)).await.is_err() {
                                    break;
                                }
                            }
                            Err(error) => {
                                let _ = incoming_tx.send(Err(error)).await;
                                break;
                            }
                        }
                    }
                }
            }
        });
        tonic::transport::Server::builder()
            .add_service(service.into_server())
            .serve_with_incoming(tokio_stream::wrappers::ReceiverStream::new(incoming_rx))
            .await
            .expect("grpc test server should run");
        accept_task.await.expect("grpc accept task should join");
    });

    (addr.to_string(), shutdown_tx, server)
}

#[tokio::test]
async fn test_set_subagent_metadata_rpc_persists_child_linkage() {
    let service = test_worker_service().await;
    let parent = create_grpc_test_session(&service).await;
    let child = create_grpc_test_session(&service).await;
    let parent_id = parent.id.clone().expect("parent id");
    let child_id = child.id.clone().expect("child id");

    let updated = service
        .set_subagent_metadata(Request::new(SetSubagentMetadataRequest {
            org_id: everruns_core::DEFAULT_ORG_ID,
            session_id: Some(child_id),
            parent_session_id: Some(parent_id.clone()),
            subagent_name: "Test Runner".to_string(),
            subagent_task: "Run the focused gRPC regression".to_string(),
            subagent_status: "running".to_string(),
        }))
        .await
        .expect("set_subagent_metadata should succeed")
        .into_inner()
        .session
        .expect("updated session");

    assert_eq!(updated.parent_session_id, Some(parent_id));
    assert_eq!(updated.subagent_name.as_deref(), Some("Test Runner"));
    assert_eq!(
        updated.subagent_task.as_deref(),
        Some("Run the focused gRPC regression")
    );
    assert_eq!(updated.subagent_status.as_deref(), Some("running"));
}

#[tokio::test]
async fn test_worker_grpc_platform_store_sets_subagent_metadata() {
    use everruns_core::platform_store::PlatformStore;

    let service = test_worker_service().await;
    let parent = create_grpc_test_session(&service).await;
    let child = create_grpc_test_session(&service).await;
    let parent_id = proto_session_id(&parent);
    let child_id = proto_session_id(&child);

    let (addr, shutdown_tx, server) = start_grpc_test_server(service).await;

    let client = everruns_worker::GrpcClient::connect(&addr)
        .await
        .expect("worker grpc client should connect");
    let platform_store =
        everruns_worker::grpc_adapters::GrpcOrgAdapter::new(client, everruns_core::DEFAULT_ORG_ID);

    let updated = platform_store
        .set_subagent_metadata(
            child_id,
            parent_id,
            "Handoff Worker",
            "Complete a worker-path handoff",
            everruns_core::session::SubagentStatus::Running,
        )
        .await
        .expect("worker platform store should set metadata over grpc");

    assert_eq!(updated.parent_session_id, Some(parent_id));
    assert_eq!(updated.subagent_name.as_deref(), Some("Handoff Worker"));
    assert_eq!(
        updated.subagent_task.as_deref(),
        Some("Complete a worker-path handoff")
    );
    assert_eq!(
        updated.subagent_status,
        Some(everruns_core::session::SubagentStatus::Running)
    );

    let _ = shutdown_tx.send(());
    server.await.expect("grpc server task should join");
}

#[tokio::test]
async fn test_subagent_and_handoff_tools_complete_over_grpc_platform_adapter() {
    use everruns_core::capabilities::{AgentHandoffCapability, Capability, SubagentCapability};
    use everruns_core::platform_store::PlatformStore;
    use everruns_core::tools::ToolExecutionResult;

    let service = test_worker_service_with_completing_runner().await;
    let parent = create_grpc_test_session(&service).await;
    let parent_id = proto_session_id(&parent);
    let user = service
        .db
        .create_user(crate::storage::models::CreateUserRow {
            email: format!("grpc-subagent-{}@example.com", uuid::Uuid::now_v7()),
            name: "gRPC Subagent Test".to_string(),
            avatar_url: None,
            roles: vec!["admin".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("create test user");
    service
        .db
        .add_organization_member(everruns_core::DEFAULT_ORG_ID, user.id, "owner")
        .await
        .expect("add test user to default org");
    service
        .db
        .update_session(
            everruns_core::DEFAULT_ORG_ID,
            parent_id,
            crate::storage::models::UpdateSession {
                resolved_owner_user_id: everruns_durable::UpdateField::Set(user.id),
                ..Default::default()
            },
        )
        .await
        .expect("mark parent session as user owned");
    let parent_harness_id = parent
        .harness_id
        .as_ref()
        .expect("parent harness id")
        .value
        .parse()
        .map(everruns_core::HarnessId::from_uuid)
        .expect("harness uuid");

    let (addr, shutdown_tx, server) = start_grpc_test_server(service).await;
    let client = everruns_worker::GrpcClient::connect(&addr)
        .await
        .expect("worker grpc client should connect");
    let adapter = Arc::new(
        everruns_worker::grpc_adapters::GrpcOrgAdapter::new_for_platform_session(
            client.clone(),
            everruns_core::DEFAULT_ORG_ID,
            Some(parent_id),
        ),
    );
    let mut context = everruns_core::traits::ToolContext::new(parent_id);
    context.platform_store = Some(adapter.clone());
    context.session_store = Some(adapter.clone());

    let spawn_tool = SubagentCapability
        .tools()
        .into_iter()
        .find(|tool| tool.name() == "spawn_subagent")
        .expect("spawn_subagent tool");
    let spawn_result = spawn_tool
        .execute_with_context(
            serde_json::json!({
                "name": "gRPC Subagent",
                "instructions": "Exercise spawn_subagent through the gRPC platform adapter"
            }),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(spawn_value) = spawn_result else {
        panic!("spawn_subagent should succeed over grpc, got {spawn_result:?}");
    };
    assert_eq!(spawn_value["status"], "idle");
    assert_eq!(spawn_value["result"], "Child completed through gRPC");
    let subagent_id: everruns_core::SessionId = spawn_value["subagent_id"]
        .as_str()
        .expect("subagent_id")
        .parse()
        .expect("subagent id parses");
    let subagent = adapter
        .get_session_by_id(subagent_id)
        .await
        .expect("get spawned subagent")
        .expect("spawned subagent exists");
    assert_eq!(subagent.parent_session_id, Some(parent_id));
    assert_eq!(subagent.subagent_name.as_deref(), Some("gRPC Subagent"));
    assert_eq!(
        subagent.subagent_status,
        Some(everruns_core::session::SubagentStatus::Completed)
    );

    let target_agent = adapter
        .create_agent(
            "grpc-handoff-target",
            Some("gRPC Handoff Target"),
            None,
            "You complete test handoffs.",
            &[],
        )
        .await
        .expect("create handoff target agent");
    let handoff_config = serde_json::json!({
        "targets": [{
            "id": "target",
            "name": "gRPC Handoff Target",
            "agent_id": target_agent.public_id,
            "harness_id": parent_harness_id,
            "required_connections": ["fake_aws"],
            "required_scopes": ["fake_aws:rds:create"]
        }]
    });
    let handoff_tool = AgentHandoffCapability
        .tools_with_config(&handoff_config)
        .into_iter()
        .find(|tool| tool.name() == "start_agent_handoff")
        .expect("start_agent_handoff tool");

    context.connection_resolver = Some(Arc::new(AllowingConnectionResolver));
    let handoff_result = handoff_tool
        .execute_with_context(
            serde_json::json!({
                "target": "target",
                "instructions": "Exercise start_agent_handoff through the gRPC platform adapter",
                "public_context": { "ticket": "EVE-538" }
            }),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(handoff_value) = handoff_result else {
        panic!("start_agent_handoff should succeed over grpc, got {handoff_result:?}");
    };
    assert_eq!(handoff_value["status"], "idle");
    assert_eq!(handoff_value["result"], "Child completed through gRPC");
    let handoff_id: everruns_core::SessionId = handoff_value["handoff_id"]
        .as_str()
        .expect("handoff_id")
        .parse()
        .expect("handoff id parses");
    let handoff = adapter
        .get_session_by_id(handoff_id)
        .await
        .expect("get handoff child")
        .expect("handoff child exists");
    assert_eq!(handoff.parent_session_id, Some(parent_id));
    assert_eq!(
        handoff.subagent_name.as_deref(),
        Some("gRPC Handoff Target")
    );
    assert_eq!(
        handoff.subagent_status,
        Some(everruns_core::session::SubagentStatus::Completed)
    );

    let _ = shutdown_tx.send(());
    server.await.expect("grpc server task should join");
}

#[tokio::test]
async fn test_execute_command_uses_user_permissions() {
    use crate::storage::models::CreateUserRow;

    let service = test_worker_service().await;
    let user = service
        .db
        .create_user(CreateUserRow {
            email: "grpc-member@example.com".to_string(),
            name: "gRPC Member".to_string(),
            avatar_url: None,
            external_id: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("test".to_string()),
            auth_provider_id: None,
        })
        .await
        .expect("create user");
    service
        .db
        .ensure_membership(user.id, everruns_core::DEFAULT_ORG_ID, "member")
        .await
        .expect("ensure membership");

    let response = service
        .execute_command(Request::new(ExecuteCommandRequest {
            name: "create_harness".to_string(),
            api_version: "v1".to_string(),
            params_json: serde_json::to_vec(&serde_json::json!({
                "name": "forbidden-from-grpc",
                "system_prompt": "prompt",
                "tags": [],
                "capabilities": [],
                "initial_files": [],
                "mcp_servers": {},
            }))
            .expect("serialize params"),
            org_id: everruns_core::DEFAULT_ORG_ID,
            user_id: Some(user.id.to_string()),
            idempotency_key: None,
            metadata: Default::default(),
        }))
        .await
        .expect("execute_command should return a structured command error")
        .into_inner();

    let proto::execute_command_response::Result::Error(error) =
        response.result.expect("command result should be present")
    else {
        panic!("expected Error response");
    };

    assert_eq!(error.kind, 2);
    assert!(error.message.contains("Access denied"));
}

/// Acquire env lock, tolerating poison from #[should_panic] tests.
fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// TM-DURABLE-002: require_grpc_auth_token panics when WORKER_GRPC_AUTH_TOKEN is unset
#[test]
#[should_panic(expected = "WORKER_GRPC_AUTH_TOKEN must be set")]
fn test_require_grpc_auth_token_panics_without_env() {
    let _lock = lock_env();
    unsafe { std::env::remove_var("WORKER_GRPC_AUTH_TOKEN") };
    require_grpc_auth_token();
}

#[test]
fn test_require_grpc_auth_token_returns_value() {
    let _lock = lock_env();
    unsafe { std::env::set_var("WORKER_GRPC_AUTH_TOKEN", "test-token-123") };
    let token = require_grpc_auth_token();
    assert_eq!(token, "test-token-123");
    unsafe { std::env::remove_var("WORKER_GRPC_AUTH_TOKEN") };
}

#[test]
fn test_grpc_server_tls_returns_none_when_no_env_vars() {
    let _lock = lock_env();
    unsafe {
        std::env::remove_var("WORKER_GRPC_TLS_CERT");
        std::env::remove_var("WORKER_GRPC_TLS_KEY");
        std::env::remove_var("WORKER_GRPC_TLS_CA_CERT");
    }
    let config = grpc_server_tls_from_env();
    assert!(
        config.is_none(),
        "Should return None when TLS not configured"
    );
}

#[test]
fn test_grpc_server_tls_returns_none_when_cert_empty() {
    let _lock = lock_env();
    unsafe {
        std::env::set_var("WORKER_GRPC_TLS_CERT", "");
        std::env::set_var("WORKER_GRPC_TLS_KEY", "");
    }
    let config = grpc_server_tls_from_env();
    assert!(config.is_none());
    unsafe {
        std::env::remove_var("WORKER_GRPC_TLS_CERT");
        std::env::remove_var("WORKER_GRPC_TLS_KEY");
    }
}

#[test]
fn test_grpc_server_tls_returns_config_with_valid_certs() {
    let _lock = lock_env();
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let cert_path = format!("{}/tests/fixtures/test-server-cert.pem", manifest);
    let key_path = format!("{}/tests/fixtures/test-server-key.pem", manifest);

    unsafe {
        std::env::set_var("WORKER_GRPC_TLS_CERT", &cert_path);
        std::env::set_var("WORKER_GRPC_TLS_KEY", &key_path);
        std::env::remove_var("WORKER_GRPC_TLS_CA_CERT");
    }

    let config = grpc_server_tls_from_env();
    assert!(
        config.is_some(),
        "Should return Some when cert+key are configured"
    );

    unsafe {
        std::env::remove_var("WORKER_GRPC_TLS_CERT");
        std::env::remove_var("WORKER_GRPC_TLS_KEY");
    }
}

#[test]
fn test_grpc_server_tls_with_client_ca() {
    let _lock = lock_env();
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let cert_path = format!("{}/tests/fixtures/test-server-cert.pem", manifest);
    let key_path = format!("{}/tests/fixtures/test-server-key.pem", manifest);
    let ca_path = format!("{}/tests/fixtures/test-ca.pem", manifest);

    unsafe {
        std::env::set_var("WORKER_GRPC_TLS_CERT", &cert_path);
        std::env::set_var("WORKER_GRPC_TLS_KEY", &key_path);
        std::env::set_var("WORKER_GRPC_TLS_CA_CERT", &ca_path);
    }

    let config = grpc_server_tls_from_env();
    assert!(
        config.is_some(),
        "Should return Some when cert+key+ca are configured"
    );

    unsafe {
        std::env::remove_var("WORKER_GRPC_TLS_CERT");
        std::env::remove_var("WORKER_GRPC_TLS_KEY");
        std::env::remove_var("WORKER_GRPC_TLS_CA_CERT");
    }
}

#[test]
#[should_panic(expected = "Failed to read WORKER_GRPC_TLS_CERT")]
fn test_grpc_server_tls_panics_on_missing_cert_file() {
    let _lock = lock_env();
    unsafe {
        std::env::set_var("WORKER_GRPC_TLS_CERT", "/nonexistent/cert.pem");
        std::env::set_var("WORKER_GRPC_TLS_KEY", "/nonexistent/key.pem");
    }
    let _config = grpc_server_tls_from_env();
    // cleanup won't run due to panic, but that's fine for test
}

// ========================================================================
// Session SQL database helper tests
// ========================================================================

#[test]
fn test_sqldb_error_to_status_maps_not_found() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::DatabaseNotFound("test_db".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::NotFound);
}

#[test]
fn test_sqldb_error_to_status_maps_already_exists() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::DatabaseAlreadyExists("test_db".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::AlreadyExists);
}

#[test]
fn test_sqldb_error_to_status_maps_invalid_name() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::InvalidDatabaseName("bad!name".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

#[test]
fn test_sqldb_error_to_status_maps_limit_exceeded() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::LimitExceeded("max 10".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::ResourceExhausted);
}

#[test]
fn test_sqldb_error_to_status_maps_query_error() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::QueryError("syntax error".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
}

#[test]
fn test_sqldb_error_to_status_maps_timeout() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::QueryTimeout(30);
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
}

#[test]
fn test_sqldb_error_to_status_maps_authorizer_blocked() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::AuthorizerBlocked("DROP TABLE".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::PermissionDenied);
}

#[test]
fn test_sqldb_error_to_status_maps_result_too_large() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::ResultTooLarge("1MB limit".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
}

#[test]
fn test_sqldb_error_to_status_maps_internal() {
    use everruns_core::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::Internal("unexpected".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::Internal);
}

#[test]
fn test_json_value_to_proto_null() {
    let val = json_value_to_proto(serde_json::Value::Null);
    assert!(matches!(
        val.kind,
        Some(prost_types::value::Kind::NullValue(0))
    ));
}

#[test]
fn test_json_value_to_proto_string() {
    let val = json_value_to_proto(serde_json::Value::String("hello".into()));
    assert!(matches!(val.kind, Some(prost_types::value::Kind::StringValue(ref s)) if s == "hello"));
}

#[test]
fn test_json_value_to_proto_number() {
    let val = json_value_to_proto(serde_json::json!(42.0));
    assert!(
        matches!(val.kind, Some(prost_types::value::Kind::NumberValue(n)) if (n - 42.0).abs() < f64::EPSILON)
    );
}

#[test]
fn test_json_value_to_proto_bool() {
    let val = json_value_to_proto(serde_json::Value::Bool(true));
    assert!(matches!(
        val.kind,
        Some(prost_types::value::Kind::BoolValue(true))
    ));
}

#[test]
fn test_json_value_to_proto_array() {
    let val = json_value_to_proto(serde_json::json!([1, "two", null]));
    match val.kind {
        Some(prost_types::value::Kind::ListValue(list)) => {
            assert_eq!(list.values.len(), 3);
            assert!(matches!(
                list.values[0].kind,
                Some(prost_types::value::Kind::NumberValue(_))
            ));
            assert!(matches!(
                list.values[1].kind,
                Some(prost_types::value::Kind::StringValue(_))
            ));
            assert!(matches!(
                list.values[2].kind,
                Some(prost_types::value::Kind::NullValue(_))
            ));
        }
        _ => panic!("Expected ListValue"),
    }
}

#[test]
fn test_json_value_to_proto_object() {
    let val = json_value_to_proto(serde_json::json!({"key": "value", "num": 42}));
    match val.kind {
        Some(prost_types::value::Kind::StructValue(s)) => {
            assert_eq!(s.fields.len(), 2);
            assert!(s.fields.contains_key("key"));
            assert!(s.fields.contains_key("num"));
        }
        _ => panic!("Expected StructValue"),
    }
}

#[test]
fn test_db_info_to_proto_roundtrip() {
    use chrono::Utc;
    let now = Utc::now();
    let info = everruns_core::session_sqldb::DatabaseInfo {
        name: "test_db".into(),
        size_bytes: 4096,
        page_count: 1,
        created_at: now,
        updated_at: now,
    };
    let proto = db_info_to_proto(info);
    assert_eq!(proto.name, "test_db");
    assert_eq!(proto.size_bytes, 4096);
    assert_eq!(proto.page_count, 1);
    assert!(proto.created_at.is_some());
    assert!(proto.updated_at.is_some());
}
