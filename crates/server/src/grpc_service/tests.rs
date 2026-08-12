use super::*;
use tonic::service::Interceptor;

// Env-var-mutating tests must not run in parallel.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const EXAMPLE_TOKEN: &str = "YExample0";

async fn test_worker_service() -> WorkerServiceImpl {
    test_worker_service_with_runner(None).await
}

async fn test_worker_service_with_runner(
    runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
) -> WorkerServiceImpl {
    let db = Arc::new(StorageBackend::in_memory());
    let grade = everruns_core::DeploymentGrade::Dev;
    let host_composition = crate::oss_host_composition_for_grade(grade);
    let encryption = Some(Arc::new(
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
            .expect("valid test encryption key"),
    ));

    crate::seed::seed_all(&db, grade, &crate::seed::SeedAuthContext::default())
        .await
        .expect("seed test data");

    let event_service =
        EventService::with_listeners(db.clone(), crate::EventDelivery::in_memory(), vec![]);

    WorkerServiceImpl::new(event_service, db, encryption, runner, host_composition)
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
        // Emit the terminal turn event the production completion path always
        // emits (session_lifecycle::turn_completed). Adapters derive the child's
        // settled status from this event, not from the bare `idle` status.
        self.event_service
            .emit(everruns_core::EventRequest::new(
                session_id,
                everruns_core::events::EventContext::empty(),
                everruns_core::events::TurnCompletedData {
                    turn_id: everruns_core::typed_id::TurnId::new(),
                    iterations: 1,
                    duration_ms: None,
                    usage: None,
                    input_content: None,
                    final_message_id: None,
                    final_answer_preview: None,
                    time_to_first_token_ms: None,
                    tool_call_count: None,
                    llm_call_count: None,
                    status: None,
                },
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
    let host_composition = crate::oss_host_composition_for_grade(grade);
    let encryption = Some(Arc::new(
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
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
        host_composition,
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
    let mut interceptor = GrpcAuthInterceptor::new(Some(EXAMPLE_TOKEN.to_string()));
    let mut request = Request::new(());
    request
        .metadata_mut()
        .insert("authorization", "Bearer YExample0".parse().unwrap());
    assert!(interceptor.call(request).is_ok());
}

#[test]
fn test_interceptor_rejects_missing_token() {
    let mut interceptor = GrpcAuthInterceptor::new(Some(EXAMPLE_TOKEN.to_string()));
    let request = Request::new(());
    let err = interceptor.call(request).unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    assert!(err.message().contains("Missing"));
}

#[test]
fn test_interceptor_rejects_wrong_token() {
    let mut interceptor = GrpcAuthInterceptor::new(Some(EXAMPLE_TOKEN.to_string()));
    let mut request = Request::new(());
    request
        .metadata_mut()
        .insert("authorization", "Bearer wrong_token".parse().unwrap());
    let err = interceptor.call(request).unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    assert!(err.message().contains("Invalid"));
}

#[test]
fn test_interceptor_rejects_same_length_wrong_token() {
    // Same length as the expected token so the constant-time compare reaches
    // its byte-comparison branch rather than short-circuiting on length.
    let mut interceptor = GrpcAuthInterceptor::new(Some(EXAMPLE_TOKEN.to_string()));
    let mut request = Request::new(());
    request
        .metadata_mut()
        .insert("authorization", "Bearer YExample1".parse().unwrap());
    let err = interceptor.call(request).unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    assert!(err.message().contains("Invalid"));
}

#[test]
fn test_interceptor_rejects_non_bearer_scheme() {
    let mut interceptor = GrpcAuthInterceptor::new(Some(EXAMPLE_TOKEN.to_string()));
    let mut request = Request::new(());
    request
        .metadata_mut()
        .insert("authorization", "Basic YExample0".parse().unwrap());
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

struct DenyGrpcSessionManageResolver;

impl everruns_core::PermissionResolver for DenyGrpcSessionManageResolver {
    fn has_permission(
        &self,
        caller: &everruns_core::Caller,
        permission: &everruns_core::Permission,
    ) -> bool {
        *permission != everruns_core::Permission::OrgSessionsManage
            && everruns_core::DefaultPermissionResolver.has_permission(caller, permission)
    }

    fn caller_permissions(&self, caller: &everruns_core::Caller) -> Vec<everruns_core::Permission> {
        everruns_core::DefaultPermissionResolver
            .caller_permissions(caller)
            .into_iter()
            .filter(|permission| *permission != everruns_core::Permission::OrgSessionsManage)
            .collect()
    }
}

#[tokio::test]
async fn authorize_session_creation_is_owner_scoped_and_returns_budget_root() {
    use crate::storage::models::{CreateSessionRow, CreateUserRow};

    let mut service = test_worker_service().await;
    let user = service
        .db
        .create_user(CreateUserRow {
            email: "grpc-session-authority@example.com".to_string(),
            name: "gRPC Session Authority".to_string(),
            avatar_url: None,
            external_id: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("test".to_string()),
            auth_provider_id: None,
        })
        .await
        .unwrap();
    service
        .db
        .ensure_membership(user.id, everruns_core::DEFAULT_ORG_ID, "owner")
        .await
        .unwrap();
    let session = service
        .db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: everruns_core::DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_identity_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: Some(user.id),
            title: Some("authority root".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
        })
        .await
        .unwrap();

    let response = service
        .authorize_session_creation(Request::new(AuthorizeSessionCreationRequest {
            org_id: session.org_id,
            session_id: session.id.to_string(),
        }))
        .await
        .expect("owner authority")
        .into_inner();
    assert_eq!(response.budget_root_session_id, session.id.to_string());

    let foreign = service
        .authorize_session_creation(Request::new(AuthorizeSessionCreationRequest {
            org_id: session.org_id + 1,
            session_id: session.id.to_string(),
        }))
        .await
        .expect_err("cross-org authority lookup must fail");
    assert_eq!(foreign.code(), tonic::Code::NotFound);

    service.set_permission_resolver(Arc::new(DenyGrpcSessionManageResolver));
    let denied = service
        .authorize_session_creation(Request::new(AuthorizeSessionCreationRequest {
            org_id: session.org_id,
            session_id: session.id.to_string(),
        }))
        .await
        .expect_err("active permission resolver must be honored");
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);
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
async fn test_subagent_and_handoff_tools_complete_over_grpc_platform_adapter() {
    use everruns_core::tools::{Tool, ToolExecutionResult};
    use everruns_platform::PlatformStore;

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
    context.subagent_delegate = Some(std::sync::Arc::new(
        everruns_platform::PlatformStoreSubagentDelegate(adapter.clone()),
    ));
    context
        .extensions
        .insert(std::sync::Arc::new(everruns_platform::PlatformStoreExt(
            adapter.clone(),
        )));
    context.session_store = Some(adapter.clone());
    context.session_creation_authority = Some(Arc::new(
        everruns_worker::grpc_adapters::GrpcSessionCreationAuthority::new(
            client.clone(),
            everruns_core::DEFAULT_ORG_ID,
            parent_id,
        ),
    ));

    let spawn_tool = everruns_platform::capabilities::SpawnSubagentAsAgentTool;
    let spawn_result = spawn_tool
        .execute_with_context(
            serde_json::json!({
                "name": "gRPC Subagent",
                "instructions": "Exercise spawn_agent subagent delegation through the gRPC platform adapter",
                "target": { "type": "subagent" },
                "mode": "foreground"
            }),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(spawn_value) = spawn_result else {
        panic!("spawn_agent subagent target should succeed over grpc, got {spawn_result:?}");
    };
    assert_eq!(spawn_value["status"], "completed");
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
    // parent_session_id is set on spawn for delegation tree tracking; name/status now live on the task record.
    assert_eq!(subagent.parent_session_id, Some(parent_id));

    let detached_result = spawn_tool
        .execute_with_context(
            serde_json::json!({
                "name": "gRPC Detached Peer",
                "instructions": "Exercise authorized detached creation through the worker path",
                "target": { "type": "subagent" },
                "lifetime": "detached",
                "mode": "foreground"
            }),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(detached_value) = detached_result else {
        panic!("authorized detached spawn should succeed over grpc, got {detached_result:?}");
    };
    let detached_id: everruns_core::SessionId = detached_value["subagent_id"]
        .as_str()
        .expect("detached id")
        .parse()
        .expect("detached id parses");
    let detached = adapter
        .get_session_by_id(detached_id)
        .await
        .expect("get detached peer")
        .expect("detached peer exists");
    assert_eq!(detached.parent_session_id, None);
    assert_eq!(detached.forked_from_session_id, Some(parent_id));

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
    let handoff_tool = everruns_platform::capabilities::SpawnAgentHandoffTool::new(&handoff_config);

    context.connection_resolver = Some(Arc::new(AllowingConnectionResolver));
    let handoff_result = handoff_tool
        .execute_with_context(
            serde_json::json!({
                "name": "gRPC Handoff Target",
                "instructions": "Exercise spawn_agent handoff delegation through the gRPC platform adapter",
                "target": { "type": "agent", "id": "target" },
                "mode": "foreground",
                "public_context": { "ticket": "EVE-538" }
            }),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(handoff_value) = handoff_result else {
        panic!("spawn_agent handoff target should succeed over grpc, got {handoff_result:?}");
    };
    assert_eq!(handoff_value["status"], "completed");
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
    // parent_session_id is set on spawn; name/status now live on the task record.
    assert_eq!(handoff.parent_session_id, Some(parent_id));

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

#[tokio::test]
async fn platform_command_surface_uses_session_owner_and_org() {
    use crate::storage::models::{CreateSessionRow, CreateUserRow};

    let service = test_worker_service().await;
    let user = service
        .db
        .create_user(CreateUserRow {
            email: "platform-surface-member@example.com".to_string(),
            name: "Platform Surface Member".to_string(),
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
    let session = service
        .db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: everruns_core::DEFAULT_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_identity_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(2),
            resolved_owner_user_id: Some(user.id),
            title: Some("platform surface".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
        })
        .await
        .expect("create session");

    let response = service
        .invoke_platform_command_surface(Request::new(InvokePlatformCommandSurfaceRequest {
            session_id: Some(proto::Uuid {
                value: session.id.uuid().to_string(),
            }),
            org_id: session.org_id,
            operation: PlatformCommandSurfaceOperation::Discover as i32,
            arguments_json: serde_json::to_vec(&serde_json::json!({
                "query": "models"
            }))
            .unwrap(),
        }))
        .await
        .expect("discover succeeds")
        .into_inner();
    let proto::invoke_platform_command_surface_response::Result::Output(output) =
        response.result.expect("discover result")
    else {
        panic!("expected discover output");
    };
    assert!(output.contains("list_models"));

    for (query, expected) in [
        ("create agent", ["create_agent", "default_model_id"]),
        (
            "create agent trigger",
            ["create_agent_trigger", "cron_expression"],
        ),
    ] {
        let response = service
            .invoke_platform_command_surface(Request::new(InvokePlatformCommandSurfaceRequest {
                session_id: Some(proto::Uuid {
                    value: session.id.uuid().to_string(),
                }),
                org_id: session.org_id,
                operation: PlatformCommandSurfaceOperation::Discover as i32,
                arguments_json: serde_json::to_vec(&serde_json::json!({ "query": query })).unwrap(),
            }))
            .await
            .expect("command discovery succeeds")
            .into_inner();
        let proto::invoke_platform_command_surface_response::Result::Output(output) =
            response.result.expect("discover result")
        else {
            panic!("expected discover output");
        };
        for needle in expected {
            assert!(
                output.contains(needle),
                "{query} discovery omitted {needle}"
            );
        }
    }

    let denied = service
        .invoke_platform_command_surface(Request::new(InvokePlatformCommandSurfaceRequest {
            session_id: Some(proto::Uuid {
                value: session.id.uuid().to_string(),
            }),
            org_id: session.org_id,
            operation: PlatformCommandSurfaceOperation::Execute as i32,
            arguments_json: serde_json::to_vec(&serde_json::json!({
                "commands": "create_harness --name forbidden"
            }))
            .unwrap(),
        }))
        .await
        .expect("authorization denial is a tool result")
        .into_inner();
    let proto::invoke_platform_command_surface_response::Result::Error(error) =
        denied.result.expect("execute result")
    else {
        panic!("member mutation must be denied");
    };
    assert!(
        error.contains("forbidden") || error.contains("Access denied"),
        "unexpected authorization error: {error}"
    );

    let foreign = service
        .invoke_platform_command_surface(Request::new(InvokePlatformCommandSurfaceRequest {
            session_id: Some(proto::Uuid {
                value: session.id.uuid().to_string(),
            }),
            org_id: session.org_id + 1,
            operation: PlatformCommandSurfaceOperation::Discover as i32,
            arguments_json: br#"{"query":"models"}"#.to_vec(),
        }))
        .await
        .expect_err("cross-org session lookup must fail");
    assert_eq!(foreign.code(), tonic::Code::NotFound);
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
fn test_require_grpc_auth_token_result_errors_without_env() {
    let _lock = lock_env();
    unsafe { std::env::remove_var("WORKER_GRPC_AUTH_TOKEN") };

    let error = require_grpc_auth_token_result().expect_err("missing token should error");
    assert!(
        error
            .to_string()
            .contains("WORKER_GRPC_AUTH_TOKEN must be set")
    );
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
fn test_grpc_server_tls_result_errors_on_missing_cert_file() {
    let _lock = lock_env();
    unsafe {
        std::env::set_var("WORKER_GRPC_TLS_CERT", "/nonexistent/cert.pem");
        std::env::set_var("WORKER_GRPC_TLS_KEY", "/nonexistent/key.pem");
        std::env::remove_var("WORKER_GRPC_TLS_CA_CERT");
    }

    let error = grpc_server_tls_from_env_result().expect_err("missing cert should error");
    assert!(
        error
            .to_string()
            .contains("Failed to read WORKER_GRPC_TLS_CERT")
    );

    unsafe {
        std::env::remove_var("WORKER_GRPC_TLS_CERT");
        std::env::remove_var("WORKER_GRPC_TLS_KEY");
    }
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
    use everruns_platform::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::DatabaseNotFound("test_db".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::NotFound);
}

#[test]
fn test_sqldb_error_to_status_maps_already_exists() {
    use everruns_platform::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::DatabaseAlreadyExists("test_db".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::AlreadyExists);
}

#[test]
fn test_sqldb_error_to_status_maps_invalid_name() {
    use everruns_platform::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::InvalidDatabaseName("bad!name".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

#[test]
fn test_sqldb_error_to_status_maps_limit_exceeded() {
    use everruns_platform::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::LimitExceeded("max 10".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::ResourceExhausted);
}

#[test]
fn test_sqldb_error_to_status_maps_query_error() {
    use everruns_platform::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::QueryError("syntax error".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
}

#[test]
fn test_sqldb_error_to_status_maps_timeout() {
    use everruns_platform::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::QueryTimeout(30);
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
}

#[test]
fn test_sqldb_error_to_status_maps_authorizer_blocked() {
    use everruns_platform::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::AuthorizerBlocked("DROP TABLE".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::PermissionDenied);
}

#[test]
fn test_sqldb_error_to_status_maps_result_too_large() {
    use everruns_platform::session_sqldb::SessionSqlDbError;
    let err = SessionSqlDbError::ResultTooLarge("1MB limit".into());
    let status = sqldb_error_to_status(err);
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
}

#[test]
fn test_sqldb_error_to_status_maps_internal() {
    use everruns_platform::session_sqldb::SessionSqlDbError;
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
    let info = everruns_platform::session_sqldb::DatabaseInfo {
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

// ============================================================================
// Session task RPC tests — ListOrphanedSessionTasks
// ============================================================================

/// Helper: create a session task via the storage backend directly.
async fn create_test_session_task(
    db: &crate::storage::StorageBackend,
    session_id: everruns_core::SessionId,
    kind: &str,
    state: everruns_core::session_task::SessionTaskState,
    wake_policy: everruns_core::session_task::TaskWakePolicy,
) -> String {
    use everruns_core::session_task::{CreateSessionTask, TaskLinks, new_session_task};
    let task = new_session_task(
        CreateSessionTask {
            session_id,
            id: None,
            kind: kind.to_string(),
            display_name: "Test task".to_string(),
            spec: serde_json::json!({}),
            state,
            links: TaskLinks::default(),
            wake_policy,
        },
        chrono::Utc::now(),
    );
    db.create_session_task(&task)
        .await
        .expect("create task")
        .0
        .id
}

#[tokio::test]
async fn test_list_orphaned_session_tasks_returns_stale_task() {
    let svc = test_worker_service().await;
    let db = svc.db.clone();

    let session_id = everruns_core::SessionId::new();

    let task_id = create_test_session_task(
        &db,
        session_id,
        everruns_core::session_task::TASK_KIND_BACKGROUND_TOOL,
        everruns_core::session_task::SessionTaskState::Running,
        everruns_core::session_task::TaskWakePolicy::Silent,
    )
    .await;

    // Backdate the heartbeat to be older than the stale threshold (5 minutes).
    let stale_heartbeat = chrono::Utc::now() - chrono::Duration::minutes(10);
    db.update_session_task(
        session_id,
        &task_id,
        everruns_core::session_task::SessionTaskUpdate {
            heartbeat_at: Some(stale_heartbeat),
            ..Default::default()
        },
    )
    .await
    .expect("set stale heartbeat");

    // Call the RPC with a 5-minute stale threshold.
    let request = tonic::Request::new(ListOrphanedSessionTasksRequest {
        stale_after_seconds: 300,
        limit: 50,
    });
    let response = svc
        .list_orphaned_session_tasks(request)
        .await
        .expect("rpc should succeed");

    let entries = response.into_inner().entries;
    let found = entries.iter().any(|e| e.task_id == task_id);
    assert!(
        found,
        "Expected task {} in orphan list, got: {:?}",
        task_id,
        entries.iter().map(|e| &e.task_id).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_list_orphaned_session_tasks_excludes_fresh_heartbeat() {
    let svc = test_worker_service().await;
    let db = svc.db.clone();

    let session_id = everruns_core::SessionId::new();

    let task_id = create_test_session_task(
        &db,
        session_id,
        everruns_core::session_task::TASK_KIND_BACKGROUND_TOOL,
        everruns_core::session_task::SessionTaskState::Running,
        everruns_core::session_task::TaskWakePolicy::Silent,
    )
    .await;

    // Set a recent heartbeat (only 1 second ago).
    let fresh_heartbeat = chrono::Utc::now() - chrono::Duration::seconds(1);
    db.update_session_task(
        session_id,
        &task_id,
        everruns_core::session_task::SessionTaskUpdate {
            heartbeat_at: Some(fresh_heartbeat),
            ..Default::default()
        },
    )
    .await
    .expect("set fresh heartbeat");

    // Call with a 5-minute stale threshold — fresh task must not appear.
    let request = tonic::Request::new(ListOrphanedSessionTasksRequest {
        stale_after_seconds: 300,
        limit: 50,
    });
    let response = svc
        .list_orphaned_session_tasks(request)
        .await
        .expect("rpc should succeed");

    let entries = response.into_inner().entries;
    let found = entries.iter().any(|e| e.task_id == task_id);
    assert!(
        !found,
        "Fresh-heartbeat task {} must not be in orphan list",
        task_id
    );
}

#[tokio::test]
async fn test_list_orphaned_session_tasks_excludes_null_heartbeat() {
    let svc = test_worker_service().await;
    let db = svc.db.clone();

    let session_id = everruns_core::SessionId::new();

    // Create a task with no heartbeat (foreground/subagent tasks).
    let task_id = create_test_session_task(
        &db,
        session_id,
        everruns_core::session_task::TASK_KIND_SUBAGENT,
        everruns_core::session_task::SessionTaskState::Running,
        everruns_core::session_task::TaskWakePolicy::OnTerminal,
    )
    .await;

    // heartbeat_at is NULL — must never appear in orphan list.
    let request = tonic::Request::new(ListOrphanedSessionTasksRequest {
        stale_after_seconds: 1,
        limit: 50,
    });
    let response = svc
        .list_orphaned_session_tasks(request)
        .await
        .expect("rpc should succeed");

    let entries = response.into_inner().entries;
    let found = entries.iter().any(|e| e.task_id == task_id);
    assert!(
        !found,
        "NULL-heartbeat task {} must not be in orphan list",
        task_id
    );
}
