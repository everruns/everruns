//! Hosted Work Chat must qualify its effective Agent and confine consumers.
use crate::agent_api_channel_test::{ApiChannel, call};
use crate::test_harness::TestServer;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

async fn setup(server: &TestServer, capabilities: Value, mode: &str) -> ApiChannel {
    let agent:Value=server.post("/v1/agents",json!({"name":format!("work-{}",uuid::Uuid::new_v4().simple()),"system_prompt":"Help with work","capabilities":capabilities})).await.assert_status(StatusCode::CREATED).json();
    let agent_id = agent["id"].as_str().unwrap().to_string();
    let channel:Value=server.post(&format!("/v1/agents/{agent_id}/channels"),json!({"channel_type":"work_chat","channel_config":{"interface_mode":mode,"execution":{"visibility":"full"}}})).await.assert_status(StatusCode::CREATED).json();
    ApiChannel {
        agent_id,
        channel_id: channel["id"].as_str().unwrap().to_string(),
    }
}
async fn publish(server: &TestServer, channel: &ApiChannel) -> crate::test_harness::TestResponse {
    server
        .post(
            &format!(
                "/v1/agents/{}/channels/{}/publish",
                channel.agent_id, channel.channel_id
            ),
            json!({}),
        )
        .await
}

#[tokio::test]
async fn work_chat_publish_requires_effective_coordinator_tools() {
    let server = TestServer::in_memory().await;
    let channel = setup(&server, json!([]), "single_with_threads").await;
    publish(&server, &channel)
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    let config: Value = server
        .get(&format!(
            "/v1/agents/{}/channels/{}",
            channel.agent_id, channel.channel_id
        ))
        .await
        .assert_success()
        .json();
    assert_eq!(config["status"], "draft");
    let coordinator = setup(
        &server,
        json!([{"ref":"coordination","config":{"role":"coordinator","workers":[{"id":"self"}]}}]),
        "single_with_threads",
    )
    .await;
    publish(&server, &coordinator).await.assert_success();
    server
        .patch(
            &format!("/v1/agents/{}", coordinator.agent_id),
            json!({"capabilities":[]}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    let config: Value = server
        .get(&format!(
            "/v1/agents/{}/chat-features",
            coordinator.agent_id
        ))
        .await
        .assert_success()
        .json();
    assert_eq!(config["threads"], true, "Rejected update must roll back");
}

#[tokio::test]
async fn work_chat_multi_chat_needs_no_coordination_and_rejects_unauthenticated_sessions() {
    let server = TestServer::in_memory().await;
    let channel = setup(&server, json!([]), "multi_chat").await;
    publish(&server, &channel).await.assert_success();
    let base = format!("/v1/channels/{}", channel.channel_id);
    let bootstrap: Value = server
        .get(&format!("{base}/chat/config"))
        .await
        .assert_success()
        .json();
    assert_eq!(bootstrap["interface_mode"], "multi_chat");
    assert_eq!(bootstrap["features"]["threads"], false);
    call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        None,
        Some(json!({})),
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
    assert!(
        bootstrap.get("execution").is_none(),
        "No provider secrets in public bootstrap"
    );
}

#[tokio::test]
async fn worker_role_does_not_qualify_for_thread_interface() {
    let server = TestServer::in_memory().await;
    let channel = setup(
        &server,
        json!([{"ref":"coordination","config":{"role":"worker"}}]),
        "single_with_threads",
    )
    .await;
    publish(&server, &channel)
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

async fn consumer_token(server: &TestServer, channel: &ApiChannel, subject: &str) -> String {
    let user = server
        .db
        .resolve_runtime_identity(
            everruns_server::storage::runtime_identity::VerifiedRuntimeIdentity {
                org_id: everruns_core::DEFAULT_ORG_ID,
                provider: "oidc".into(),
                realm: "work-chat-test".into(),
                subject: subject.into(),
                name: subject.into(),
                avatar_url: None,
                management_user_id: None,
            },
        )
        .await
        .unwrap();
    let bindings = server
        .db
        .list_virtual_user_bindings(everruns_core::DEFAULT_ORG_ID, user.id)
        .await
        .unwrap();
    everruns_server::auth::jwt::JwtService::new(everruns_server::auth::AuthConfig::default().jwt)
        .generate_runtime_token(
            everruns_core::DEFAULT_ORG_ID,
            user.id,
            channel.channel_id.clone(),
            bindings[0].id,
        )
        .unwrap()
}
#[tokio::test]
async fn work_chat_consumers_cannot_read_other_users_channels_or_management() {
    let server = TestServer::in_memory().await;
    let channel = setup(&server, json!([]), "multi_chat").await;
    let other = setup(&server, json!([]), "multi_chat").await;
    publish(&server, &channel).await.assert_success();
    publish(&server, &other).await.assert_success();
    let alice = consumer_token(&server, &channel, "alice").await;
    let bob = consumer_token(&server, &channel, "bob").await;
    let base = format!("/v1/channels/{}", channel.channel_id);
    let session: Value = call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        Some(&alice),
        Some(json!({})),
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    call(
        &server,
        Method::GET,
        &format!("{base}/sessions/{}", session["id"].as_str().unwrap()),
        Some(&bob),
        None,
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
    call(
        &server,
        Method::GET,
        &format!("/v1/channels/{}/sessions", other.channel_id),
        Some(&alice),
        None,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
    // This fixture intentionally runs AUTH_MODE=none, which grants anonymous
    // management access. Check the production JWT boundary directly instead.
    let jwt = everruns_server::auth::jwt::JwtService::new(
        everruns_server::auth::AuthConfig::default().jwt,
    );
    assert!(jwt.validate_access_token(&alice).is_err());
}
#[tokio::test]
async fn work_chat_main_conversation_is_stable_across_concurrent_tabs() {
    let server = TestServer::in_memory().await;
    let channel = setup(
        &server,
        json!([{"ref":"coordination","config":{"role":"coordinator","workers":[{"id":"self"}]}}]),
        "single_with_threads",
    )
    .await;
    publish(&server, &channel).await.assert_success();
    let token = consumer_token(&server, &channel, "alice").await;
    let path = format!("/v1/channels/{}/chat/main", channel.channel_id);
    let (first, second) = tokio::join!(
        call(&server, Method::POST, &path, Some(&token), Some(json!({}))),
        call(&server, Method::POST, &path, Some(&token), Some(json!({})))
    );
    let first: Value = first.assert_success().json();
    let second: Value = second.assert_success().json();
    assert_eq!(first["id"], second["id"]);
    call(
        &server,
        Method::POST,
        &format!("/v1/channels/{}/sessions", channel.channel_id),
        Some(&token),
        Some(json!({})),
    )
    .await
    .assert_status(StatusCode::CONFLICT);
}

#[tokio::test]
async fn work_chat_worker_link_delegates_only_inside_the_consumer_tree() {
    use everruns_internal_protocol::{proto, uuid_to_proto_uuid};
    use proto::worker_service_server::WorkerService;
    let server = TestServer::in_memory().await;
    let channel = setup(
        &server,
        json!([{"ref":"coordination","config":{"role":"coordinator","workers":[{"id":"self"}]}}]),
        "single_with_threads",
    )
    .await;
    publish(&server, &channel).await.assert_success();
    let token = consumer_token(&server, &channel, "alice").await;
    let root: Value = call(
        &server,
        Method::POST,
        &format!("/v1/channels/{}/chat/main", channel.channel_id),
        Some(&token),
        Some(json!({})),
    )
    .await
    .assert_success()
    .json();
    let root_id: everruns_contracts::typed_id::SessionId =
        root["id"].as_str().unwrap().parse().unwrap();
    let jwt = everruns_server::auth::jwt::JwtService::new(
        everruns_server::auth::AuthConfig::default().jwt,
    );
    let subject = jwt
        .validate_runtime_token(&token)
        .unwrap()
        .sub
        .parse()
        .unwrap();
    let input = uuid::Uuid::now_v7();
    server
        .db
        .record_runtime_invocation(
            everruns_core::DEFAULT_ORG_ID,
            root_id,
            input,
            Some(subject),
            None,
            None,
        )
        .await
        .unwrap();
    let service = everruns_server::worker_link::grpc_service::WorkerServiceImpl::new(
        (*server.event_service).clone(),
        server.db.clone(),
        server.encryption.clone(),
        Some(server.runner.clone()),
        everruns_server::oss_host_composition_for_grade(everruns_core::DeploymentGrade::Dev),
    );
    let request = |name: &str, params: Value| {
        let mut request = tonic::Request::new(proto::ExecuteCommandRequest {
            name: name.into(),
            api_version: "v1".into(),
            org_id: everruns_core::DEFAULT_ORG_ID,
            runtime_view: true,
            platform_session_id: Some(uuid_to_proto_uuid(root_id.uuid())),
            input_message_id: Some(uuid_to_proto_uuid(input)),
            params_json: serde_json::to_vec(&params).unwrap(),
            ..Default::default()
        });
        request.extensions_mut().insert(
            everruns_server::worker_link::grpc_service::authority::WorkerAuthority::Development,
        );
        request
    };
    let created = service
        .execute_command(request(
            "create_session",
            json!({"agent_id":channel.agent_id,"parent_session_id":root_id,"title":"Research"}),
        ))
        .await
        .unwrap()
        .into_inner()
        .result
        .unwrap();
    let proto::execute_command_response::Result::OkJson(created) = created else {
        panic!("thread creation failed: {created:?}")
    };
    let child: Value = serde_json::from_slice(&created).unwrap();
    let row = server
        .db
        .get_session(
            everruns_core::DEFAULT_ORG_ID,
            child["id"].as_str().unwrap().parse().unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    let parent = server
        .db
        .get_session(everruns_core::DEFAULT_ORG_ID, root_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.parent_session_id, Some(root_id));
    assert_eq!(row.channel_id, parent.channel_id);
    assert_eq!(row.owner_principal_id, parent.owner_principal_id);
    call(
        &server,
        Method::GET,
        &format!(
            "/v1/channels/{}/sessions/{}",
            channel.channel_id,
            child["id"].as_str().unwrap()
        ),
        Some(&token),
        None,
    )
    .await
    .assert_success();
    let denied = service
        .execute_command(request("create_agent", json!({"name":"Forbidden"})))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);
    let denied = service
        .execute_command(request(
            "create_session",
            json!({"agent_id":server.seed_chat_agent_id,"parent_session_id":root_id}),
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);
    let bob = consumer_token(&server, &channel, "bob").await;
    call(
        &server,
        Method::GET,
        &format!(
            "/v1/channels/{}/sessions/{}",
            channel.channel_id,
            child["id"].as_str().unwrap()
        ),
        Some(&bob),
        None,
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn work_chat_inherited_coordinator_changes_are_atomic() {
    let server = TestServer::in_memory().await;
    let harness:Value=server.post("/v1/harnesses",json!({"name":"work-coordinator","capabilities":[{"ref":"coordination","config":{"role":"coordinator","workers":[{"id":"self"}]}}]})).await.assert_success().json();
    let channel = setup(&server, json!([]), "single_with_threads").await;
    server
        .patch(
            &format!("/v1/agents/{}", channel.agent_id),
            json!({"harness_id":harness["id"]}),
        )
        .await
        .assert_success();
    publish(&server, &channel).await.assert_success();
    server
        .patch(
            &format!("/v1/harnesses/{}", harness["id"].as_str().unwrap()),
            json!({"capabilities":[]}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    let features: Value = server
        .get(&format!("/v1/agents/{}/chat-features", channel.agent_id))
        .await
        .assert_success()
        .json();
    assert_eq!(features["threads"], true);
}

#[tokio::test]
async fn work_chat_files_and_events_keep_private_context_out() {
    use everruns_server::domains::session_files::service::{CreateFileInput, WorkspaceFileService};
    let server = TestServer::in_memory().await;
    let channel = setup(
        &server,
        json!([{"ref":"session_file_system"}]),
        "multi_chat",
    )
    .await;
    publish(&server, &channel).await.assert_success();
    let token = consumer_token(&server, &channel, "alice").await;
    let base = format!("/v1/channels/{}", channel.channel_id);
    let session: Value = call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        Some(&token),
        Some(json!({})),
    )
    .await
    .assert_success()
    .json();
    let id: everruns_contracts::typed_id::SessionId =
        session["id"].as_str().unwrap().parse().unwrap();
    let files = WorkspaceFileService::new(server.db.clone());
    for (path, readonly) in [
        ("/report.md", false),
        ("/locked.md", true),
        ("/memory/private.md", false),
    ] {
        files
            .create_file(
                id.uuid(),
                CreateFileInput {
                    path: path.into(),
                    content: Some("original".into()),
                    encoding: None,
                    is_readonly: Some(readonly),
                },
            )
            .await
            .unwrap();
    }
    let mut tree = everruns_core::capability_types::VirtualFileTree::new();
    tree.insert_text("/reference/guide.md", "immutable");
    server.virtual_registry.register(
        id.uuid(),
        "/reference".into(),
        std::sync::Arc::new(tree),
        "test".into(),
    );
    // Knowledge roots reject ordinary file writes. Seed mounted trees so the
    // consumer boundary also has actual private entries to filter from listings.
    for root in ["knowledge", "knowledge-text"] {
        let mut tree = everruns_core::capability_types::VirtualFileTree::new();
        tree.insert_text(format!("/{root}/private.md"), "private Knowledge");
        server.virtual_registry.register(
            id.uuid(),
            format!("/{root}"),
            std::sync::Arc::new(tree),
            "test".into(),
        );
    }
    let url = format!("{base}/sessions/{id}/files");
    call(
        &server,
        Method::GET,
        &format!("{url}?path=/workspace/missing.txt"),
        Some(&token),
        None,
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
    let virtual_file: Value = call(
        &server,
        Method::GET,
        &format!("{url}?path=/workspace/reference/guide.md"),
        Some(&token),
        None,
    )
    .await
    .assert_success()
    .json();
    assert_eq!(virtual_file["file"]["is_readonly"], true);
    call(
        &server,
        Method::POST,
        &url,
        Some(&token),
        Some(json!({"path":"/workspace/reference/guide.md","content":"shadow"})),
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);
    call(
        &server,
        Method::POST,
        &url,
        Some(&token),
        Some(json!({"path":"/workspace/large.txt","content":"a".repeat(3*1024*1024)})),
    )
    .await
    .assert_success();
    let opened: Value = call(
        &server,
        Method::GET,
        &format!("{url}?path=/workspace/report.md"),
        Some(&token),
        None,
    )
    .await
    .assert_success()
    .json();
    call(
        &server,
        Method::POST,
        &url,
        Some(&token),
        Some(json!({"path":"/workspace/report.md","content":"updated","expected_revision":opened["revision"]})),
    )
    .await
    .assert_success();
    assert_eq!(
        files
            .read_file(id.uuid(), "/report.md")
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
        Some("updated")
    );
    assert!(
        !call(
            &server,
            Method::POST,
            &url,
            Some(&token),
            Some(json!({"path":"/workspace/locked.md","content":"forbidden"}))
        )
        .await
        .status()
        .is_success()
    );
    assert_eq!(
        files
            .read_file(id.uuid(), "/locked.md")
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
        Some("original")
    );
    for root in ["memory", "knowledge", "knowledge-text"] {
        let path = format!("/workspace/{root}/private.md");
        for method in [Method::GET, Method::DELETE] {
            call(
                &server,
                method,
                &format!("{url}?path={path}"),
                Some(&token),
                None,
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
        }
        call(
            &server,
            Method::POST,
            &url,
            Some(&token),
            Some(json!({"path":path,"content":"consumer shadow"})),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);
    }
    let listing: Value = call(&server, Method::GET, &url, Some(&token), None)
        .await
        .assert_success()
        .json();
    assert!(!listing.to_string().contains("memory"));
    assert!(!listing.to_string().contains("knowledge"));
    call(&server,Method::POST,&url,Some(&token),Some(json!({"path":"/workspace/report.md","content":"stale","expected_revision":opened["revision"]}))).await.assert_status(StatusCode::CONFLICT);
    assert_eq!(
        files
            .read_file(id.uuid(), "/report.md")
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
        Some("updated")
    );
    for (kind, data) in [
        ("reason.completed", json!({"private":"reasoning"})),
        (
            "tool.completed",
            json!({"tool_call_id":"test","tool_name":"read_file","success":false,"error":"private backend failure","result":{"secret":"hidden"}}),
        ),
    ] {
        server
            .db
            .create_event(everruns_server::storage::CreateEventRow {
                session_id: id,
                event_type: kind.into(),
                ts: chrono::Utc::now(),
                context: json!({}),
                data,
                metadata: Some(json!({"internal":"hidden"})),
                tags: None,
            })
            .await
            .unwrap();
    }
    let events: Value = call(
        &server,
        Method::GET,
        &format!("{base}/sessions/{id}/events"),
        Some(&token),
        None,
    )
    .await
    .assert_success()
    .json();
    let text = events.to_string();
    assert!(!text.contains("reasoning"));
    assert!(!text.contains("private backend"));
    assert!(!text.contains("hidden"));
}

#[tokio::test]
async fn work_chat_org_members_require_full_scope_on_both_url_spellings() {
    use crate::agent_api_spend_members_test::member_token_with_scopes;
    let server = TestServer::in_memory().await;
    let channel = setup(
        &server,
        json!([{"ref":"session_file_system"}]),
        "multi_chat",
    )
    .await;
    server.patch(&format!("/v1/agents/{}/channels/{}", channel.agent_id, channel.channel_id), json!({"channel_config":{"interface_mode":"multi_chat","execution":{"org_members":true}}})).await.assert_success();
    publish(&server, &channel).await.assert_success();
    let full = member_token_with_scopes(&server, true, &["*"]).await;
    let read = member_token_with_scopes(&server, true, &["read"]).await;
    let canonical = format!("/v1/channels/{}", channel.channel_id);
    let session: Value = call(
        &server,
        Method::POST,
        &format!("{canonical}/sessions"),
        Some(&full),
        Some(json!({})),
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let id = session["id"].as_str().unwrap();
    for prefix in ["/v1/channels", "/v1/e"] {
        let base = format!("{prefix}/{}", channel.channel_id);
        call(
            &server,
            Method::GET,
            &format!("{base}/chat/config"),
            None,
            None,
        )
        .await
        .assert_success();
        call(
            &server,
            Method::GET,
            &format!("{base}/sessions/{id}/work"),
            Some(&full),
            None,
        )
        .await
        .assert_success();
        call(
            &server,
            Method::GET,
            &format!("{base}/sessions/{id}/files?path=/workspace"),
            Some(&full),
            None,
        )
        .await
        .assert_success();
        for (method, path, body) in [
            (Method::POST, format!("{base}/chat/main"), Some(json!({}))),
            (
                Method::POST,
                format!("{base}/sessions/{id}/files"),
                Some(json!({"path":"/workspace/member.md","content":"hello"})),
            ),
            (
                Method::DELETE,
                format!("{base}/sessions/{id}/files?path=/workspace/member.md"),
                None,
            ),
        ] {
            let response = call(&server, method, &path, Some(&read), body)
                .await
                .assert_status(StatusCode::FORBIDDEN);
            let body: Value = response.json();
            assert_eq!(body["code"], "insufficient_scope", "{path}: {body}");
        }
    }
}
