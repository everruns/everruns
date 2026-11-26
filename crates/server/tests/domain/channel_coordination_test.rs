//! A coordinator answering in a Slack conversation, or running a trigger aimed
//! at one, starts threads only on its allowed workers, inside that
//! conversation, and never posts them into the channel (TM-AUTHZ-030).
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
            json!({"name": format!("slack-{}", uuid::Uuid::new_v4().simple()), "system_prompt": "Work", "capabilities": capabilities}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    agent["id"].as_str().unwrap().to_string()
}

fn command(
    session: SessionId,
    input: uuid::Uuid,
    name: &str,
    params: Value,
) -> tonic::Request<proto::ExecuteCommandRequest> {
    let mut request = tonic::Request::new(proto::ExecuteCommandRequest {
        name: name.into(),
        api_version: "v1".into(),
        org_id: everruns_core::DEFAULT_ORG_ID,
        runtime_view: true,
        platform_session_id: Some(uuid_to_proto_uuid(session.uuid())),
        input_message_id: Some(uuid_to_proto_uuid(input)),
        params_json: serde_json::to_vec(&params).unwrap(),
        ..Default::default()
    });
    request.extensions_mut().insert(
        everruns_server::worker_link::grpc_service::authority::WorkerAuthority::Development,
    );
    request
}

fn ok(result: Option<proto::execute_command_response::Result>) -> Value {
    match result.unwrap() {
        proto::execute_command_response::Result::OkJson(json) => {
            serde_json::from_slice::<Value>(&json).unwrap()
        }
        other => panic!("command failed: {other:?}"),
    }
}

#[tokio::test]
async fn a_slack_coordinator_starts_threads_only_on_its_team() {
    let server = TestServer::in_memory().await;
    let org = everruns_core::DEFAULT_ORG_ID;
    let member = agent(&server, json!([])).await;
    let coordinator = agent(
        &server,
        json!([{"ref": "threads", "config": {"agents": [{"agent_id": member}]}}]),
    )
    .await;
    let coordinator_row = server
        .db
        .get_agent_by_public_id(org, &coordinator)
        .await
        .unwrap()
        .unwrap();
    let responder = coordinator_row.id.uuid();
    let channel: Value = server
        .post(
            &format!("/v1/platform/agents/{coordinator}/channels"),
            json!({"channel_type": "slack", "channel_config": {}}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let channel_public = channel["id"].as_str().unwrap().to_string();
    let channel_internal: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM agent_channels WHERE public_id = $1")
            .bind(&channel_public)
            .fetch_one(&server.pool)
            .await
            .unwrap();
    let set_status = |status: &'static str| {
        sqlx::query("UPDATE agent_channels SET status = $1 WHERE id = $2")
            .bind(status)
            .bind(channel_internal)
    };
    set_status("live").execute(&server.pool).await.unwrap();
    // The per-channel conversation, as the Slack host creates it.
    let session: Value = server
        .post("/v1/sessions", json!({"agent_id": coordinator}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let root_id: SessionId = session["id"].as_str().unwrap().parse().unwrap();
    sqlx::query("UPDATE sessions SET channel_id = $1 WHERE id = $2")
        .bind(channel_internal)
        .bind(root_id)
        .execute(&server.pool)
        .await
        .unwrap();
    let root = server.db.get_session(org, root_id).await.unwrap().unwrap();

    // A workspace member's message: an end user, never a management user.
    let slack_member = server
        .db
        .resolve_runtime_identity(
            everruns_server::storage::runtime_identity::VerifiedRuntimeIdentity {
                org_id: org,
                provider: "slack".into(),
                realm: "T1".into(),
                subject: "U1".into(),
                name: "Mykola".into(),
                avatar_url: None,
                management_user_id: None,
            },
        )
        .await
        .unwrap();
    let from_member = uuid::Uuid::now_v7();
    server
        .db
        .record_runtime_invocation(
            org,
            root_id,
            from_member,
            Some(slack_member.id),
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

    let child = ok(service
        .execute_command(command(
            root_id,
            from_member,
            "create_session",
            json!({"agent_id": member, "parent_session_id": root_id, "title": "Bump serde"}),
        ))
        .await
        .unwrap()
        .into_inner()
        .result);
    let child_id: SessionId = child["id"].as_str().unwrap().parse().unwrap();
    let row = server.db.get_session(org, child_id).await.unwrap().unwrap();
    assert_eq!(row.parent_session_id, Some(root_id));
    assert_eq!(row.owner_principal_id, root.owner_principal_id);
    // Threads stay out of the channel: only the coordinator answers there.
    assert_eq!(row.channel_id, None);
    ok(service
        .execute_command(command(
            root_id,
            from_member,
            "get_session",
            json!({"session_id": child_id}),
        ))
        .await
        .unwrap()
        .into_inner()
        .result);

    // Outside the team, outside the conversation, or management: refused.
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
            .execute_command(command(root_id, from_member, name, params))
            .await
            .unwrap_err();
        assert_eq!(denied.code(), tonic::Code::PermissionDenied, "{name}");
    }

    // A trigger aimed at the conversation: no person, answered by the agent.
    let scheduled = uuid::Uuid::now_v7();
    server
        .db
        .record_runtime_invocation(org, root_id, scheduled, None, None, Some(responder))
        .await
        .unwrap();
    ok(service
        .execute_command(command(
            root_id,
            scheduled,
            "create_session",
            json!({"agent_id": member, "parent_session_id": root_id, "title": "Nightly"}),
        ))
        .await
        .unwrap()
        .into_inner()
        .result);

    // A channel that is no longer live grants nothing.
    set_status("disabled").execute(&server.pool).await.unwrap();
    let denied = service
        .execute_command(command(
            root_id,
            from_member,
            "create_session",
            json!({"agent_id": member, "parent_session_id": root_id}),
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);
}
