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
    for (method, code) in [("GetExtendedAgentCard", -32007)] {
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

async fn send_task(server: &TestServer, endpoint: &A2aEndpoint, id: &str) -> String {
    let sent: Value = rpc(
        server,
        endpoint,
        vec![("A2A-Version", "1.0")],
        v1_send(id, json!({})),
    )
    .await
    .json();
    sent["result"]["task"]["id"].as_str().unwrap().to_string()
}

async fn v1_call(
    server: &TestServer,
    endpoint: &A2aEndpoint,
    method: &str,
    params: Value,
) -> Value {
    rpc(
        server,
        endpoint,
        vec![("A2A-Version", "1.0")],
        json!({ "jsonrpc": "2.0", "id": method, "method": method, "params": params }),
    )
    .await
    .json()
}

/// `ListTasks` returns only this endpoint's tasks, newest first, and pages
/// with an opaque cursor.
#[tokio::test]
async fn list_tasks_pages_through_this_endpoints_tasks_only() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;
    let other = create_a2a_endpoint(&server, "session_per_invocation").await;
    let first = send_task(&server, &endpoint, "first").await;
    let second = send_task(&server, &endpoint, "second").await;
    send_task(&server, &other, "elsewhere").await;

    let all = v1_call(&server, &endpoint, "ListTasks", json!({})).await;
    let ids: Vec<&str> = all["result"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [second.as_str(), first.as_str()], "{all}");
    assert_eq!(all["result"]["totalSize"], 2);
    assert_eq!(all["result"]["nextPageToken"], "");
    assert!(all["result"]["tasks"][0].get("artifacts").is_none());

    let page1 = v1_call(&server, &endpoint, "ListTasks", json!({ "pageSize": 1 })).await;
    assert_eq!(page1["result"]["tasks"][0]["id"], second.as_str());
    let token = page1["result"]["nextPageToken"].as_str().unwrap();
    assert!(!token.is_empty(), "{page1}");
    let page2 = v1_call(
        &server,
        &endpoint,
        "ListTasks",
        json!({ "pageSize": 1, "pageToken": token }),
    )
    .await;
    assert_eq!(page2["result"]["tasks"][0]["id"], first.as_str(), "{page2}");

    let by_context = v1_call(&server, &other, "ListTasks", json!({ "contextId": first })).await;
    assert_eq!(by_context["result"]["tasks"], json!([]), "{by_context}");

    let bad = v1_call(&server, &endpoint, "ListTasks", json!({ "pageSize": 0 })).await;
    assert_eq!(bad["error"]["code"], -32602, "{bad}");
}

/// `SubscribeToTask` refuses an unknown task and a finished one, and a
/// finished task cannot be canceled again.
#[tokio::test]
async fn subscribe_and_cancel_respect_terminal_tasks() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;
    let task_id = send_task(&server, &endpoint, "to-cancel").await;

    let unknown = v1_call(
        &server,
        &endpoint,
        "SubscribeToTask",
        json!({ "id": "00000000-0000-0000-0000-000000000000" }),
    )
    .await;
    assert_eq!(unknown["error"]["code"], -32602, "{unknown}");

    let canceled = v1_call(&server, &endpoint, "CancelTask", json!({ "id": task_id })).await;
    assert_eq!(
        canceled["result"]["status"]["state"], "TASK_STATE_CANCELED",
        "{canceled}"
    );
    let again = v1_call(&server, &endpoint, "CancelTask", json!({ "id": task_id })).await;
    assert_eq!(again["error"]["code"], -32002, "{again}");
    let subscribe = v1_call(
        &server,
        &endpoint,
        "SubscribeToTask",
        json!({ "id": task_id }),
    )
    .await;
    assert_eq!(subscribe["error"]["code"], -32004, "{subscribe}");
}

/// The `status` filter keeps exactly the tasks in that state.
#[tokio::test]
async fn list_tasks_filters_on_state() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;
    let task_id = send_task(&server, &endpoint, "queued").await;

    let submitted = v1_call(
        &server,
        &endpoint,
        "ListTasks",
        json!({ "status": "TASK_STATE_SUBMITTED" }),
    )
    .await;
    assert_eq!(
        submitted["result"]["tasks"][0]["id"],
        task_id.as_str(),
        "{submitted}"
    );
    let completed = v1_call(
        &server,
        &endpoint,
        "ListTasks",
        json!({ "status": "TASK_STATE_COMPLETED" }),
    )
    .await;
    assert_eq!(completed["result"]["tasks"], json!([]), "{completed}");
}

/// Push configs round-trip through Create, Get, List and Delete, never echo
/// the receiver's secrets, and stay fenced to the endpoint that owns the task.
#[tokio::test]
async fn push_configs_round_trip_without_echoing_secrets() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;
    let other = create_a2a_endpoint(&server, "session_per_invocation").await;
    let task_id = send_task(&server, &endpoint, "push").await;
    let config = json!({
        "taskId": task_id,
        "id": "hook-1",
        "url": "https://hooks.example.com/a2a",
        "token": "receiver-token",
        "authentication": { "scheme": "Bearer", "credentials": "receiver-secret" },
    });

    let created = v1_call(
        &server,
        &endpoint,
        "CreateTaskPushNotificationConfig",
        config.clone(),
    )
    .await;
    let expected = json!({
        "id": "hook-1",
        "taskId": task_id,
        "url": "https://hooks.example.com/a2a",
        "authentication": { "scheme": "Bearer" },
    });
    assert_eq!(created["result"], expected, "{created}");
    let lookup = json!({ "taskId": task_id, "id": "hook-1" });
    let got = v1_call(
        &server,
        &endpoint,
        "GetTaskPushNotificationConfig",
        lookup.clone(),
    )
    .await;
    assert_eq!(got["result"], expected, "{got}");
    let listed = v1_call(
        &server,
        &endpoint,
        "ListTaskPushNotificationConfigs",
        json!({ "taskId": task_id }),
    )
    .await;
    assert_eq!(listed["result"]["configs"], json!([expected]), "{listed}");

    // Another endpoint's key cannot see or redirect this task's updates.
    let foreign = v1_call(&server, &other, "CreateTaskPushNotificationConfig", config).await;
    assert_eq!(foreign["error"]["code"], -32001, "{foreign}");

    let deleted = v1_call(
        &server,
        &endpoint,
        "DeleteTaskPushNotificationConfig",
        lookup.clone(),
    )
    .await;
    assert_eq!(deleted["result"], json!({}), "{deleted}");
    let gone = v1_call(&server, &endpoint, "GetTaskPushNotificationConfig", lookup).await;
    assert_eq!(gone["error"]["code"], -32001, "{gone}");

    for url in [
        "http://hooks.example.com/a2a",
        "https://169.254.169.254/latest",
    ] {
        let refused = v1_call(
            &server,
            &endpoint,
            "CreateTaskPushNotificationConfig",
            json!({ "taskId": task_id, "url": url }),
        )
        .await;
        assert_eq!(refused["error"]["code"], -32602, "{url}: {refused}");
    }
}

/// A `SendMessage` can register its push config up front, and 0.3 clients
/// manage configs with their own method names and shapes.
#[tokio::test]
async fn send_message_registers_push_config_and_0_3_names_work() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_endpoint(&server, "session_per_invocation").await;
    let sent: Value = rpc(
        &server,
        &endpoint,
        vec![("A2A-Version", "1.0")],
        v1_send(
            "with-push",
            json!({ "configuration": {
                "returnImmediately": true,
                "taskPushNotificationConfig": { "id": "on-send", "url": "https://hooks.example.com/a2a" },
            }}),
        ),
    )
    .await
    .json();
    let task_id = sent["result"]["task"]["id"].as_str().unwrap().to_string();

    let legacy = |method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": method, "method": method, "params": params });
    let listed: Value = rpc(
        &server,
        &endpoint,
        vec![],
        legacy(
            "tasks/pushNotificationConfig/list",
            json!({ "id": task_id }),
        ),
    )
    .await
    .json();
    assert_eq!(
        listed["result"],
        json!([{
            "taskId": task_id,
            "pushNotificationConfig": { "id": "on-send", "url": "https://hooks.example.com/a2a" },
        }]),
        "{listed}"
    );
    let set: Value = rpc(
        &server,
        &endpoint,
        vec![],
        legacy(
            "tasks/pushNotificationConfig/set",
            json!({ "taskId": task_id, "pushNotificationConfig": {
                "id": "legacy", "url": "https://hooks.example.com/legacy",
                "authentication": { "schemes": ["Basic"], "credentials": "dXNlcjpwdw==" },
            }}),
        ),
    )
    .await
    .json();
    assert_eq!(
        set["result"]["pushNotificationConfig"]["authentication"],
        json!({ "schemes": ["Basic"] }),
        "{set}"
    );
    let deleted: Value = rpc(
        &server,
        &endpoint,
        vec![],
        legacy(
            "tasks/pushNotificationConfig/delete",
            json!({ "id": task_id, "pushNotificationConfigId": "legacy" }),
        ),
    )
    .await
    .json();
    assert_eq!(deleted["result"], Value::Null, "{deleted}");
    assert!(deleted.get("error").is_none(), "{deleted}");
}
