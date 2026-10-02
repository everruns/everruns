//! A2A protocol conformance for endpoints created through the Agent endpoints
//! API, the way every new A2A endpoint is created. The fixtures in
//! `endpoint_a2a_integration_test.rs` seed App-era endpoints instead.

use crate::test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use test_harness::TestServer;

struct A2aEndpoint {
    id: String,
    api_key: String,
}

async fn create_agent(server: &TestServer) -> String {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("a2a-protocol-{}", uuid::Uuid::new_v4().simple()),
                "display_name": "A2A protocol agent",
                "description": "Answers A2A calls.",
                "system_prompt": "You are a brief test agent."
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    agent["id"].as_str().unwrap().to_string()
}

async fn create_a2a_endpoint(server: &TestServer, session_mode: &str) -> A2aEndpoint {
    let agent_id = create_agent(server).await;
    let api_key = format!("evra2a_{}", uuid::Uuid::new_v4().simple());
    let endpoint: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/endpoints"),
            json!({
                "channel_type": "a2a",
                "channel_config": {
                    "session_mode": session_mode,
                    "message": "{{a2a.text}}",
                    "api_key_hash": hex::encode(Sha256::digest(api_key.as_bytes())),
                    "api_key_prefix": &api_key[..15],
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = endpoint["id"].as_str().unwrap().to_string();
    server
        .post(
            &format!("/v1/agents/{agent_id}/endpoints/{id}/publish"),
            json!({}),
        )
        .await
        .assert_success();
    A2aEndpoint { id, api_key }
}

async fn rpc(
    server: &TestServer,
    endpoint: &A2aEndpoint,
    extra_headers: Vec<(&str, &str)>,
    body: Value,
) -> test_harness::TestResponse {
    let auth = format!("Bearer {}", endpoint.api_key);
    let mut headers = vec![
        ("content-type", "application/json"),
        ("authorization", auth.as_str()),
    ];
    headers.extend(extra_headers);
    server
        .request_raw(
            Method::POST,
            &format!("/v1/e/{}/a2a", endpoint.id),
            headers,
            serde_json::to_vec(&body).unwrap(),
        )
        .await
}

/// Regression: canonical `/v1/e/{endpoint_id}/a2a` routes compared the
/// endpoint's synthetic App id against a legacy App id it never had, so every
/// A2A endpoint created through the Agent endpoints API answered 404.
#[tokio::test]
async fn api_created_a2a_endpoint_serves_card_and_accepts_messages() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;

    let card: Value = server
        .get(&format!(
            "/v1/e/{}/a2a/.well-known/agent-card.json",
            endpoint.id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(card["name"], "A2A protocol agent");

    let sent: Value = rpc(
        &server,
        &endpoint,
        vec![],
        json!({
            "jsonrpc": "2.0",
            "id": "send-1",
            "method": "message/send",
            "params": {
                "message": {
                    "role": "user",
                    "messageId": "m-1",
                    "parts": [{ "kind": "text", "text": "hello" }]
                }
            }
        }),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert!(sent.get("error").is_none(), "{sent}");
    assert_eq!(sent["result"]["kind"], "task");
}

/// The same endpoint must still refuse a request without its key.
#[tokio::test]
async fn api_created_a2a_endpoint_still_requires_its_key() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;
    let wrong = A2aEndpoint {
        id: endpoint.id.clone(),
        api_key: "evra2a_wrong".to_string(),
    };
    rpc(
        &server,
        &wrong,
        vec![],
        json!({ "jsonrpc": "2.0", "id": 1, "method": "message/send", "params": {} }),
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
}

fn v1_send(id: &str, extra: Value) -> Value {
    let mut params = json!({
        "message": {
            "role": "ROLE_USER",
            "messageId": format!("m-{id}"),
            "parts": [{ "text": "hello" }]
        },
        "configuration": { "returnImmediately": true }
    });
    if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            params.insert(key.clone(), value.clone());
        }
    }
    json!({ "jsonrpc": "2.0", "id": id, "method": "SendMessage", "params": params })
}

/// The card advertises both wire versions and stays readable by 0.3 clients.
#[tokio::test]
async fn agent_card_advertises_1_0_and_0_3() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;
    let card: Value = server
        .get(&format!(
            "/v1/e/{}/a2a/.well-known/agent-card.json",
            endpoint.id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let versions: Vec<&str> = card["supportedInterfaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["protocolVersion"].as_str().unwrap())
        .collect();
    assert_eq!(versions, ["1.0", "0.3"], "{card}");
    assert_eq!(card["supportedInterfaces"][0]["protocolBinding"], "JSONRPC");
    assert!(
        card["securityRequirements"][0]["schemes"].is_object(),
        "{card}"
    );
    assert!(card.get("stateTransitionHistory").is_none());
    assert!(card["capabilities"].get("stateTransitionHistory").is_none());
    // 0.3 fields of the union card.
    assert_eq!(card["protocolVersion"], "0.3.0");
    assert_eq!(card["preferredTransport"], "JSONRPC");
    assert!(card["url"].as_str().unwrap().ends_with("/a2a"), "{card}");
}

/// A 1.0 `SendMessage` answers with the `SendMessageResponse` wrapper and
/// ProtoJSON enum names, and `GetTask` reads the same task back.
#[tokio::test]
async fn v1_send_message_and_get_task_use_1_0_shapes() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;

    let sent: Value = rpc(
        &server,
        &endpoint,
        vec![("A2A-Version", "1.0")],
        v1_send("v1-1", json!({})),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert!(sent.get("error").is_none(), "{sent}");
    let task = &sent["result"]["task"];
    assert!(task.get("kind").is_none(), "{sent}");
    let state = task["status"]["state"].as_str().unwrap();
    assert!(state.starts_with("TASK_STATE_"), "{sent}");
    let task_id = task["id"].as_str().unwrap();

    let got: Value = rpc(
        &server,
        &endpoint,
        vec![("A2A-Version", "1.0")],
        json!({ "jsonrpc": "2.0", "id": "get-1", "method": "GetTask", "params": { "id": task_id } }),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(got["result"]["id"], task_id, "{got}");
    assert!(
        got["result"]["status"]["state"]
            .as_str()
            .unwrap()
            .starts_with("TASK_STATE_"),
        "{got}"
    );
}

/// An unsupported `A2A-Version` is refused with VersionNotSupportedError.
#[tokio::test]
async fn unsupported_version_is_refused() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;
    let res: Value = rpc(
        &server,
        &endpoint,
        vec![("A2A-Version", "2.0")],
        v1_send("bad-version", json!({})),
    )
    .await
    .json();
    assert_eq!(res["error"]["code"], -32009, "{res}");
}

/// Operations we do not implement answer with their dedicated error codes.
#[tokio::test]
async fn unimplemented_operations_use_their_error_codes() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;
    for (method, code) in [
        ("ListTasks", -32004),
        ("CreateTaskPushNotificationConfig", -32003),
        ("GetExtendedAgentCard", -32007),
    ] {
        let res: Value = rpc(
            &server,
            &endpoint,
            vec![("A2A-Version", "1.0")],
            json!({ "jsonrpc": "2.0", "id": method, "method": method, "params": {} }),
        )
        .await
        .json();
        assert_eq!(res["error"]["code"], code, "{method}: {res}");
    }
}

/// Continuing a task this endpoint never created is TaskNotFoundError, not a
/// silent new session.
#[tokio::test]
async fn continuing_an_unknown_task_is_task_not_found() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;
    let res: Value = rpc(
        &server,
        &endpoint,
        vec![("A2A-Version", "1.0")],
        v1_send(
            "unknown-task",
            json!({ "message": {
                "role": "ROLE_USER",
                "messageId": "m-unknown",
                "taskId": "00000000-0000-0000-0000-000000000000",
                "parts": [{ "text": "hello again" }]
            }}),
        ),
    )
    .await
    .json();
    assert_eq!(res["error"]["code"], -32001, "{res}");
}
