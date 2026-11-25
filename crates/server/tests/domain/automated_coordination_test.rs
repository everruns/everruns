//! An automated run (a schedule or event message, no person) of a coordinator
//! starts threads only on its allowed workers, inside its own run
//! (TM-AUTHZ-028).
use crate::test_harness::TestServer;
use axum::http::StatusCode;
use everruns_contracts::typed_id::SessionId;
use everruns_internal_protocol::{proto, uuid_to_proto_uuid};
use proto::worker_service_server::WorkerService;
use serde_json::{Value, json};

async fn agent(server: &TestServer, capabilities: Value) -> String {
    let agent: Value = server
        .post(
            "/v1/platform/agents",
            json!({"name": format!("auto-{}", uuid::Uuid::new_v4().simple()), "system_prompt": "Work", "capabilities": capabilities}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    agent["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn an_automated_coordinator_run_starts_threads_only_on_its_team() {
    let server = TestServer::in_memory().await;
    let member = agent(&server, json!([])).await;
    let coordinator = agent(
        &server,
        json!([
            {"ref": "coordination", "config": {"role": "coordinator", "workers": [{"id": "self"}]}},
            {"ref": "agent_handoff", "config": {"members": [{"agent_id": member}]}}
        ]),
    )
    .await;
    let session: Value = server
        .post("/v1/sessions", json!({"agent_id": coordinator}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let root_id: SessionId = session["id"].as_str().unwrap().parse().unwrap();
    let responder = server
        .db
        .get_agent_by_public_id(everruns_core::DEFAULT_ORG_ID, &coordinator)
        .await
        .unwrap()
        .unwrap()
        .id
        .uuid();
    // A schedule message: no management user, no end user.
    let input = uuid::Uuid::now_v7();
    server
        .db
        .record_runtime_invocation(
            everruns_core::DEFAULT_ORG_ID,
            root_id,
            input,
            None,
            None,
            Some(responder),
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
    let ok = |result: Option<proto::execute_command_response::Result>| match result.unwrap() {
        proto::execute_command_response::Result::OkJson(json) => {
            serde_json::from_slice::<Value>(&json).unwrap()
        }
        other => panic!("command failed: {other:?}"),
    };

    // A team member is an allowed worker without being listed in `workers`.
    ok(service
        .execute_command(request("get_agent_harness", json!({"id": member})))
        .await
        .unwrap()
        .into_inner()
        .result);
    let child = ok(service
        .execute_command(request(
            "create_session",
            json!({"agent_id": member, "parent_session_id": root_id, "title": "Bump serde"}),
        ))
        .await
        .unwrap()
        .into_inner()
        .result);
    let child_id: SessionId = child["id"].as_str().unwrap().parse().unwrap();
    let row = server
        .db
        .get_session(everruns_core::DEFAULT_ORG_ID, child_id)
        .await
        .unwrap()
        .unwrap();
    let root = server
        .db
        .get_session(everruns_core::DEFAULT_ORG_ID, root_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.parent_session_id, Some(root_id));
    assert_eq!(row.owner_principal_id, root.owner_principal_id);
    assert_eq!(row.channel_id, None);
    ok(service
        .execute_command(request("get_session", json!({"session_id": child_id})))
        .await
        .unwrap()
        .into_inner()
        .result);

    // Outside the team, outside the run, or management: refused.
    for (name, params) in [
        (
            "create_session",
            json!({"agent_id": server.seed_chat_agent_id, "parent_session_id": root_id}),
        ),
        (
            "create_session",
            json!({"agent_id": member, "parent_session_id": child_id}),
        ),
        ("create_agent", json!({"name": "forbidden"})),
    ] {
        let denied = service
            .execute_command(request(name, params))
            .await
            .unwrap_err();
        assert_eq!(denied.code(), tonic::Code::PermissionDenied, "{name}");
    }

    // An automated run of an agent that is no coordinator gets nothing new.
    let plain = agent(&server, json!([])).await;
    let plain_session: Value = server
        .post("/v1/sessions", json!({"agent_id": plain}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let plain_id: SessionId = plain_session["id"].as_str().unwrap().parse().unwrap();
    let plain_input = uuid::Uuid::now_v7();
    let plain_responder = server
        .db
        .get_agent_by_public_id(everruns_core::DEFAULT_ORG_ID, &plain)
        .await
        .unwrap()
        .unwrap()
        .id
        .uuid();
    server
        .db
        .record_runtime_invocation(
            everruns_core::DEFAULT_ORG_ID,
            plain_id,
            plain_input,
            None,
            None,
            Some(plain_responder),
        )
        .await
        .unwrap();
    let mut denied = tonic::Request::new(proto::ExecuteCommandRequest {
        name: "create_session".into(),
        api_version: "v1".into(),
        org_id: everruns_core::DEFAULT_ORG_ID,
        runtime_view: true,
        platform_session_id: Some(uuid_to_proto_uuid(plain_id.uuid())),
        input_message_id: Some(uuid_to_proto_uuid(plain_input)),
        params_json: serde_json::to_vec(&json!({"agent_id": plain, "parent_session_id": plain_id}))
            .unwrap(),
        ..Default::default()
    });
    denied.extensions_mut().insert(
        everruns_server::worker_link::grpc_service::authority::WorkerAuthority::Development,
    );
    let denied = service.execute_command(denied).await.unwrap_err();
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);
    // The worker turns this into a tool error the agent reads (tool error
    // contract), so the reason is written for it and says what to do next.
    assert!(
        denied
            .message()
            .starts_with("This run has no person behind it")
            && denied
                .message()
                .contains("tell the person who set up this run"),
        "{}",
        denied.message()
    );
}
