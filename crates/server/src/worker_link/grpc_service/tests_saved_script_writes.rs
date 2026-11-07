//! Saved-script writes over the worker command transport (TM-TOOL-067).
//!
//! A worker writes a script for a turn. The write must be bound to that turn
//! and run as its management user under `AGENT_MANAGE`; unbound writes would
//! run as the org's internal caller and skip the policy.

use super::tests::{create_grpc_test_session, test_worker_service, trusted_request};
use super::worker::commands::test_support::execute_test_command;
use super::*;

struct Fixture {
    service: WorkerServiceImpl,
    session: everruns_contracts::typed_id::SessionId,
    agent_id: String,
}

async fn fixture() -> Fixture {
    let service = test_worker_service().await;
    let (session, harness_id) = create_grpc_test_session(&service).await;
    let agent = execute_test_command(
        &service,
        "create_agent",
        serde_json::json!({
            "name": "script-owner", "harness_id": harness_id, "system_prompt": "Rules"
        }),
    )
    .await;
    Fixture {
        service,
        session,
        agent_id: agent["id"].as_str().expect("agent id").to_string(),
    }
}

fn script_params(agent_id: &str, body: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "agent_id": agent_id, "name": "triage", "description": "Label PRs.", "body": body
    }))
    .unwrap()
}

async fn create_script(
    f: &Fixture,
    bound_message: Option<uuid::Uuid>,
) -> Result<proto::execute_command_response::Result, tonic::Status> {
    f.service
        .execute_command(trusted_request(ExecuteCommandRequest {
            name: "create_agent_script".to_string(),
            api_version: "v1".to_string(),
            params_json: script_params(&f.agent_id, "echo planted"),
            org_id: everruns_core::DEFAULT_ORG_ID,
            platform_session_id: bound_message
                .map(|_| everruns_internal_protocol::uuid_to_proto_uuid(f.session.uuid())),
            input_message_id: bound_message.map(everruns_internal_protocol::uuid_to_proto_uuid),
            reason: Some("Saved script 'triage' from the agent's shell".to_string()),
            ..Default::default()
        }))
        .await
        .map(|response| response.into_inner().result.expect("command result"))
}

async fn script_count(f: &Fixture) -> usize {
    execute_test_command(
        &f.service,
        "list_agent_scripts",
        serde_json::json!({"agent_id": f.agent_id}),
    )
    .await
    .as_array()
    .map(Vec::len)
    .unwrap_or_default()
}

async fn member(f: &Fixture, email: &str, role: &str) -> uuid::Uuid {
    let user = f
        .service
        .db
        .create_user(crate::storage::CreateUserRow {
            email: email.to_string(),
            name: email.to_string(),
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
    f.service
        .db
        .ensure_membership(user.id, everruns_core::DEFAULT_ORG_ID, role)
        .await
        .expect("membership");
    user.id
}

#[tokio::test]
async fn unbound_script_write_is_refused_as_internal_caller() {
    let f = fixture().await;
    for name in [
        "create_agent_script",
        "update_agent_script",
        "delete_agent_script",
    ] {
        let status = f
            .service
            .execute_command(trusted_request(ExecuteCommandRequest {
                name: name.to_string(),
                api_version: "v1".to_string(),
                params_json: script_params(&f.agent_id, "echo planted"),
                org_id: everruns_core::DEFAULT_ORG_ID,
                ..Default::default()
            }))
            .await
            .expect_err("unbound script write must be refused");
        assert_eq!(status.code(), tonic::Code::PermissionDenied, "{name}");
    }
    assert_eq!(script_count(&f).await, 0, "nothing was written");
}

#[tokio::test]
async fn channel_turn_without_management_user_cannot_write_scripts() {
    let f = fixture().await;
    let message = uuid::Uuid::now_v7();
    // A channel, consumer or anonymous turn records no management user.
    f.service
        .db
        .record_runtime_invocation(
            everruns_core::DEFAULT_ORG_ID,
            f.session,
            message,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let status = create_script(&f, Some(message))
        .await
        .expect_err("turn without a management user must be refused");
    assert_eq!(status.code(), tonic::Code::PermissionDenied);
    assert_eq!(script_count(&f).await, 0);
}

#[tokio::test]
async fn management_authorized_turn_saves_script_as_its_user() {
    let f = fixture().await;
    let user = member(&f, "script-admin@example.com", "admin").await;
    let subject = f
        .service
        .db
        .default_virtual_user(everruns_core::DEFAULT_ORG_ID, user)
        .await
        .unwrap();
    let message = uuid::Uuid::now_v7();
    f.service
        .db
        .record_runtime_invocation(
            everruns_core::DEFAULT_ORG_ID,
            f.session,
            message,
            Some(subject.id),
            Some(user),
            None,
        )
        .await
        .unwrap();
    let result = create_script(&f, Some(message))
        .await
        .expect("management-authorized write");
    let proto::execute_command_response::Result::OkJson(json) = result else {
        panic!("script save failed: {result:?}");
    };
    let saved: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(saved["name"], "triage");
    assert_eq!(script_count(&f).await, 1);
}
