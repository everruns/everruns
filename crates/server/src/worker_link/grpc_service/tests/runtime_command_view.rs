//! Runtime response projection, inheritance, and mutation guards.

use super::*;

async fn execute_runtime_view(
    service: &WorkerServiceImpl,
    name: &str,
    params: serde_json::Value,
) -> proto::execute_command_response::Result {
    service
        .execute_command(Request::new(ExecuteCommandRequest {
            runtime_view: true,
            name: name.to_string(),
            api_version: "v1".to_string(),
            params_json: serde_json::to_vec(&params).unwrap(),
            org_id: everruns_core::DEFAULT_ORG_ID,
            ..Default::default()
        }))
        .await
        .expect("runtime command transport")
        .into_inner()
        .result
        .expect("runtime command result")
}

fn runtime_view_json(result: proto::execute_command_response::Result) -> serde_json::Value {
    match result {
        proto::execute_command_response::Result::OkJson(json) => {
            serde_json::from_slice(&json).unwrap()
        }
        proto::execute_command_response::Result::Error(error) => {
            panic!("runtime view failed: {error:?}")
        }
    }
}

#[tokio::test]
async fn runtime_command_views_project_records_and_fold_harness_inheritance() {
    let service = test_worker_service().await;
    let parent = execute_test_command(&service, "create_harness", serde_json::json!({
        "name": "runtime-parent", "system_prompt": "Parent rules", "capabilities": [{"ref": "session"}]
    })).await;
    let child = execute_test_command(&service, "create_harness", serde_json::json!({
        "name": "runtime-child", "system_prompt": "Child rules", "parent_harness_id": parent["id"]
    })).await;
    let params = serde_json::json!({"id": child["id"]});
    let raw = execute_test_command(&service, "get_harness", params.clone()).await;
    assert!(
        raw.get("created_at").is_some(),
        "management response stays record-shaped"
    );
    let runtime = runtime_view_json(execute_runtime_view(&service, "get_harness", params).await);
    let definition: everruns_core::HarnessDefinition =
        serde_json::from_value(runtime.clone()).unwrap();
    assert!(
        definition
            .system_prompt
            .as_deref()
            .unwrap()
            .contains("Parent rules")
    );
    assert!(
        definition
            .system_prompt
            .as_deref()
            .unwrap()
            .contains("Child rules")
    );
    assert!(
        definition
            .capabilities
            .iter()
            .any(|capability| capability.capability_id() == "session")
    );
    for field in [
        "id",
        "parent_harness_id",
        "created_at",
        "status",
        "is_built_in",
    ] {
        assert!(
            runtime.get(field).is_none(),
            "runtime harness must not expose {field}"
        );
    }
    let agent = execute_test_command(
        &service,
        "create_agent",
        serde_json::json!({
            "name": "runtime-agent", "harness_id": child["id"], "system_prompt": "Agent rules"
        }),
    )
    .await;
    let runtime = runtime_view_json(
        execute_runtime_view(
            &service,
            "get_agent",
            serde_json::json!({"id": agent["id"]}),
        )
        .await,
    );
    let _: everruns_core::AgentDefinition = serde_json::from_value(runtime.clone()).unwrap();
    for field in [
        "internal_id",
        "harness_id",
        "created_at",
        "status",
        "service_virtual_user_id",
    ] {
        assert!(
            runtime.get(field).is_none(),
            "runtime agent must not expose {field}"
        );
    }
    let runtime = runtime_view_json(
        execute_runtime_view(
            &service,
            "create_session",
            serde_json::json!({
                "harness_id": child["id"], "agent_id": agent["id"], "title": "Portable session"
            }),
        )
        .await,
    );
    let session: everruns_core::ExecutionSession = serde_json::from_value(runtime.clone()).unwrap();
    for field in [
        "owner_principal_id",
        "virtual_user_id",
        "created_at",
        "source",
        "activity",
        "participants",
    ] {
        assert!(
            runtime.get(field).is_none(),
            "runtime session must not expose {field}"
        );
    }
    let fetched = runtime_view_json(
        execute_runtime_view(
            &service,
            "get_session",
            serde_json::json!({"session_id": session.id}),
        )
        .await,
    );
    assert_eq!(fetched["id"], runtime["id"]);
    let participant = runtime_view_json(
        execute_runtime_view(
            &service,
            "add_session_participant",
            serde_json::json!({
                "session_id": session.id, "kind": "agent", "agent_id": agent["id"]
            }),
        )
        .await,
    );
    assert!(
        participant.is_string(),
        "participant projection is only its correlation ID"
    );
    let _: everruns_contracts::typed_id::SessionParticipantId =
        serde_json::from_value(participant).unwrap();
}

#[tokio::test]
async fn runtime_response_view_rejects_unsupported_commands_before_dispatch() {
    let service = test_worker_service().await;
    let result = execute_runtime_view(
        &service,
        "create_harness",
        serde_json::json!({"name": "must-not-be-created"}),
    )
    .await;
    let proto::execute_command_response::Result::Error(error) = result else {
        panic!("unsupported projection executed");
    };
    assert_eq!(error.kind, 1);
    let harnesses = execute_test_command(&service, "list_harnesses", serde_json::json!({})).await;
    assert!(
        !harnesses
            .as_array()
            .unwrap()
            .iter()
            .any(|harness| harness["name"] == "must-not-be-created")
    );
}

#[tokio::test]
async fn runtime_harness_view_rejects_archived_leaf() {
    let service = test_worker_service().await;
    let harness = execute_test_command(
        &service,
        "create_harness",
        serde_json::json!({"name": "runtime-archived"}),
    )
    .await;
    execute_test_command(
        &service,
        "update_harness",
        serde_json::json!({"id": harness["id"], "status": "archived"}),
    )
    .await;
    let result = execute_runtime_view(
        &service,
        "get_harness",
        serde_json::json!({"id": harness["id"]}),
    )
    .await;
    let proto::execute_command_response::Result::Error(error) = result else {
        panic!("archived harness resolved for execution");
    };
    assert!(error.message.contains("archived"), "{error:?}");
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
            ..Default::default()
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

    assert!(names.contains(&"generic"));
    assert!(!names.contains(&"platform-chat"));
}

#[tokio::test]
async fn runtime_harness_view_rejects_cycles_and_missing_ancestors() {
    let service = test_worker_service().await;
    let parent = execute_test_command(
        &service,
        "create_harness",
        serde_json::json!({"name": "cycle-parent"}),
    )
    .await;
    let child = execute_test_command(
        &service,
        "create_harness",
        serde_json::json!({"name": "cycle-child", "parent_harness_id": parent["id"]}),
    )
    .await;
    let parent_id: everruns_contracts::typed_id::HarnessId =
        serde_json::from_value(parent["id"].clone()).unwrap();
    let child_id: everruns_contracts::typed_id::HarnessId =
        serde_json::from_value(child["id"].clone()).unwrap();
    // Corrupt legacy data directly: normal writes reject inheritance cycles.
    service
        .db
        .update_harness(
            everruns_core::DEFAULT_ORG_ID,
            parent_id,
            crate::storage::models::UpdateHarness {
                parent_harness_id: Some(Some(child_id)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let result =
        execute_runtime_view(&service, "get_harness", serde_json::json!({"id": child_id})).await;
    let proto::execute_command_response::Result::Error(error) = result else {
        panic!("cycle resolved");
    };
    assert!(error.message.contains("cycle"), "{error:?}");
    // The foreign key forbids a dangling parent, so write one past it.
    sqlx::query("UPDATE harnesses SET parent_harness_id = $2 WHERE id = $1")
        .bind(child_id.uuid())
        .bind(uuid::Uuid::now_v7())
        .execute(&mut service.db.unchecked_connection().await)
        .await
        .unwrap();
    let result =
        execute_runtime_view(&service, "get_harness", serde_json::json!({"id": child_id})).await;
    let proto::execute_command_response::Result::Error(error) = result else {
        panic!("missing ancestor resolved");
    };
    assert_eq!(
        error.kind, 3,
        "missing ancestors keep lookup not-found behavior"
    );
}

#[tokio::test]
async fn runtime_harness_view_cannot_resolve_an_ancestor_in_another_org() {
    let service = test_worker_service().await;
    let foreign = service
        .execute_command(Request::new(ExecuteCommandRequest {
            name: "create_harness".to_string(),
            api_version: "v1".to_string(),
            params_json: br#"{"name":"foreign-parent","system_prompt":"Private rules"}"#.to_vec(),
            org_id: everruns_core::DEFAULT_ORG_ID + 1,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    let foreign = runtime_view_json(foreign.result.unwrap());
    let child = execute_test_command(
        &service,
        "create_harness",
        serde_json::json!({"name": "local-child"}),
    )
    .await;
    // Legacy corruption must not turn inheritance resolution into a tenant bypass.
    service
        .db
        .update_harness(
            everruns_core::DEFAULT_ORG_ID,
            serde_json::from_value(child["id"].clone()).unwrap(),
            crate::storage::models::UpdateHarness {
                parent_harness_id: Some(Some(
                    serde_json::from_value(foreign["id"].clone()).unwrap(),
                )),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let result = execute_runtime_view(
        &service,
        "get_harness",
        serde_json::json!({"id": child["id"]}),
    )
    .await;
    let proto::execute_command_response::Result::Error(error) = result else {
        panic!("foreign ancestor resolved");
    };
    assert_eq!(error.kind, 3, "foreign ancestors remain unavailable");
}

#[tokio::test]
async fn runtime_projection_keeps_bound_invocation_authorization() {
    let service = test_worker_service().await;
    let (session_id, harness_id) = create_grpc_test_session(&service).await;
    let status = service
        .execute_command(Request::new(ExecuteCommandRequest {
            runtime_view: true,
            name: "get_harness".to_string(),
            api_version: "v1".to_string(),
            params_json: serde_json::to_vec(&serde_json::json!({"id": harness_id})).unwrap(),
            org_id: everruns_core::DEFAULT_ORG_ID,
            platform_session_id: Some(everruns_internal_protocol::uuid_to_proto_uuid(
                session_id.uuid(),
            )),
            input_message_id: Some(everruns_internal_protocol::uuid_to_proto_uuid(
                uuid::Uuid::new_v4(),
            )),
            // A worker-supplied identity cannot override the bound invocation.
            user_id: Some("forged-owner".to_string()),
            ..Default::default()
        }))
        .await
        .expect_err("unrecorded invocation must not acquire management authority");
    assert_eq!(status.code(), tonic::Code::PermissionDenied);
}
