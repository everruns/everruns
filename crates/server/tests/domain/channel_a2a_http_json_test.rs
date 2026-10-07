//! The A2A 1.0 HTTP+JSON binding (spec §11) on an A2A channel: the same
//! operations as JSON-RPC under REST paths below the channel's A2A URL, with
//! `google.rpc.Status` errors. One test drives it with the a2a-rs REST client
//! over a real socket, the way an outside agent would.

use crate::test_harness;

use a2a_client::Transport;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use test_harness::{TestResponse, TestServer};

struct A2aEndpoint {
    id: String,
    api_key: String,
}

async fn create_a2a_channel(server: &TestServer) -> A2aEndpoint {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("a2a-http-json-{}", uuid::Uuid::new_v4().simple()),
                "display_name": "A2A HTTP+JSON agent",
                "description": "Answers A2A calls.",
                "system_prompt": "You are a brief test agent."
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent_id = agent["id"].as_str().unwrap();
    let api_key = format!("evra2a_{}", uuid::Uuid::new_v4().simple());
    let channel: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/channels"),
            json!({
                "channel_type": "a2a",
                "channel_config": {
                    "session_mode": "session_per_invocation",
                    "message": "{{a2a.text}}",
                    "api_key_hash": hex::encode(Sha256::digest(api_key.as_bytes())),
                    "api_key_prefix": &api_key[..15],
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = channel["id"].as_str().unwrap().to_string();
    server
        .post(
            &format!("/v1/agents/{agent_id}/channels/{id}/publish"),
            json!({}),
        )
        .await
        .assert_success();
    A2aEndpoint { id, api_key }
}

async fn call(
    server: &TestServer,
    endpoint: &A2aEndpoint,
    method: Method,
    route: &str,
    extra_headers: Vec<(&str, &str)>,
    body: Option<Value>,
) -> TestResponse {
    let auth = format!("Bearer {}", endpoint.api_key);
    let mut headers = vec![("authorization", auth.as_str())];
    if body.is_some() {
        headers.push(("content-type", "application/a2a+json"));
    }
    headers.extend(extra_headers);
    server
        .request_raw(
            method,
            &format!("/v1/channels/{}/a2a/{route}", endpoint.id),
            headers,
            body.map(|b| serde_json::to_vec(&b).unwrap())
                .unwrap_or_default(),
        )
        .await
}

fn send_body(text: &str) -> Value {
    json!({
        "message": {
            "role": "ROLE_USER",
            "messageId": uuid::Uuid::new_v4().to_string(),
            "parts": [{ "text": text }]
        },
        "configuration": { "returnImmediately": true }
    })
}

/// A well-formed task id that names no task: a real one with its last digit
/// changed.
fn missing_task_id(real: &str) -> String {
    let (head, last) = real.split_at(real.len() - 1);
    format!("{head}{}", if last == "0" { "1" } else { "0" })
}

fn assert_a2a_error(response: TestResponse, status: StatusCode, reason: &str) -> Value {
    let response = response.assert_status(status);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/a2a+json"
    );
    let body: Value = response.json();
    assert_eq!(body["error"]["code"], status.as_u16(), "{body}");
    assert_eq!(
        body["error"]["details"][0],
        json!({
            "@type": "type.googleapis.com/google.rpc.ErrorInfo",
            "reason": reason,
            "domain": "a2a-protocol.org",
        }),
        "{body}"
    );
    body
}

/// The card lists an HTTP+JSON 1.0 interface on the same URL as JSON-RPC.
#[tokio::test]
async fn agent_card_advertises_http_json_on_the_same_url() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_channel(&server).await;
    let card: Value = server
        .get(&format!(
            "/v1/channels/{}/a2a/.well-known/agent-card.json",
            endpoint.id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let interfaces = card["supportedInterfaces"].as_array().unwrap();
    let http_json: Vec<&Value> = interfaces
        .iter()
        .filter(|i| i["protocolBinding"] == "HTTP+JSON")
        .collect();
    assert_eq!(http_json.len(), 1, "{card}");
    assert_eq!(http_json[0]["protocolVersion"], "1.0");
    assert_eq!(http_json[0]["url"], interfaces[0]["url"]);
}

/// Send, get and list return the bare 1.0 objects as `application/a2a+json`.
#[tokio::test]
async fn send_get_and_list_return_bare_1_0_objects() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_channel(&server).await;

    let sent = call(
        &server,
        &endpoint,
        Method::POST,
        "message:send",
        vec![],
        Some(send_body("hello")),
    )
    .await
    .assert_status(StatusCode::OK);
    assert_eq!(
        sent.headers().get("content-type").unwrap(),
        "application/a2a+json"
    );
    let sent: Value = sent.json();
    assert!(sent.get("jsonrpc").is_none(), "{sent}");
    let task_id = sent["task"]["id"].as_str().unwrap().to_string();
    assert_eq!(sent["task"]["status"]["state"], "TASK_STATE_SUBMITTED");

    let task: Value = call(
        &server,
        &endpoint,
        Method::GET,
        &format!("tasks/{task_id}"),
        vec![],
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(task["id"], task_id);
    assert!(task.get("kind").is_none(), "{task}");

    let listed: Value = call(
        &server,
        &endpoint,
        Method::GET,
        "tasks?pageSize=1",
        vec![],
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(listed["pageSize"], 1, "{listed}");
    assert_eq!(listed["tasks"][0]["id"], task_id);
}

/// A2A errors use the `google.rpc.Status` envelope with the spec §5.4 status.
#[tokio::test]
async fn errors_use_the_google_rpc_status_envelope() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_channel(&server).await;
    let sent: Value = call(
        &server,
        &endpoint,
        Method::POST,
        "message:send",
        vec![],
        Some(send_body("hello")),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    let missing = missing_task_id(sent["task"]["id"].as_str().unwrap());

    let body = assert_a2a_error(
        call(
            &server,
            &endpoint,
            Method::GET,
            &format!("tasks/{missing}"),
            vec![],
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        "TASK_NOT_FOUND",
    );
    assert_eq!(body["error"]["status"], "NOT_FOUND");

    assert_a2a_error(
        call(
            &server,
            &endpoint,
            Method::POST,
            &format!("tasks/{missing}:cancel"),
            vec![],
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        "TASK_NOT_FOUND",
    );
    assert_a2a_error(
        call(
            &server,
            &endpoint,
            Method::GET,
            "tasks?pageSize=0",
            vec![],
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_PARAMS",
    );
    assert_a2a_error(
        call(
            &server,
            &endpoint,
            Method::GET,
            "extendedAgentCard",
            vec![],
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "EXTENDED_AGENT_CARD_NOT_CONFIGURED",
    );
    assert_a2a_error(
        call(
            &server,
            &endpoint,
            Method::POST,
            "message:send",
            vec![],
            Some(json!(["not", "an", "object"])),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_PARAMS",
    );
}

/// The binding is A2A 1.0 only: no header means 1.0, anything else is refused.
#[tokio::test]
async fn only_a2a_1_0_is_served() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_channel(&server).await;
    for version in ["0.3", "2.0"] {
        assert_a2a_error(
            call(
                &server,
                &endpoint,
                Method::POST,
                "message:send",
                vec![("A2A-Version", version)],
                Some(send_body("hello")),
            )
            .await,
            StatusCode::BAD_REQUEST,
            "VERSION_NOT_SUPPORTED",
        );
    }
    call(
        &server,
        &endpoint,
        Method::POST,
        "message:send",
        vec![("A2A-Version", "1.0")],
        Some(send_body("hello")),
    )
    .await
    .assert_status(StatusCode::OK);
}

/// Authentication runs before anything else, and unknown routes are plain 404s.
#[tokio::test]
async fn auth_and_routing_match_the_json_rpc_binding() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_channel(&server).await;
    let wrong = A2aEndpoint {
        id: endpoint.id.clone(),
        api_key: "evra2a_wrong".to_string(),
    };
    call(
        &server,
        &wrong,
        Method::POST,
        "message:send",
        vec![],
        Some(json!("not even an object")),
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
    call(
        &server,
        &endpoint,
        Method::POST,
        "tasks/x:archive",
        vec![],
        None,
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
    call(
        &server,
        &endpoint,
        Method::GET,
        "message:send",
        vec![],
        None,
    )
    .await
    .assert_status(StatusCode::METHOD_NOT_ALLOWED);
}

/// Push configs round-trip through the REST routes without echoing secrets.
#[tokio::test]
async fn push_configs_round_trip_over_rest_paths() {
    let server = TestServer::in_memory().await;
    let endpoint = create_a2a_channel(&server).await;
    let sent: Value = call(
        &server,
        &endpoint,
        Method::POST,
        "message:send",
        vec![],
        Some(send_body("hello")),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    let task_id = sent["task"]["id"].as_str().unwrap().to_string();
    let configs = format!("tasks/{task_id}/pushNotificationConfigs");

    let created: Value = call(
        &server,
        &endpoint,
        Method::POST,
        &configs,
        vec![],
        Some(json!({
            "id": "c1",
            "url": "https://hooks.example.com/a2a",
            "token": "receiver-token",
        })),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(created["id"], "c1", "{created}");
    assert_eq!(created["taskId"], task_id);
    assert!(created.get("token").is_none(), "{created}");

    let listed: Value = call(&server, &endpoint, Method::GET, &configs, vec![], None)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(listed["configs"][0]["id"], "c1", "{listed}");
    call(
        &server,
        &endpoint,
        Method::GET,
        &format!("{configs}/c1"),
        vec![],
        None,
    )
    .await
    .assert_status(StatusCode::OK);
    call(
        &server,
        &endpoint,
        Method::DELETE,
        &format!("{configs}/c1"),
        vec![],
        None,
    )
    .await
    .assert_status(StatusCode::OK);
    assert_a2a_error(
        call(
            &server,
            &endpoint,
            Method::GET,
            &format!("{configs}/c1"),
            vec![],
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        "TASK_NOT_FOUND",
    );
}

/// An outside agent using the a2a-rs REST client over a real socket.
#[tokio::test]
async fn a2a_rest_client_talks_to_the_channel() {
    let (server, base_url) = TestServer::serving_in_memory().await;
    let endpoint = create_a2a_channel(&server).await;
    let transport = a2a_client::rest::RestTransport::new(
        reqwest::Client::new(),
        format!("{base_url}/api/v1/channels/{}/a2a", endpoint.id),
    );
    let params = a2a_client::ServiceParams::from([(
        "Authorization".to_string(),
        vec![format!("Bearer {}", endpoint.api_key)],
    )]);

    let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("hello")]);
    message.message_id = "m-rest-1".to_string();
    let response = transport
        .send_message(
            &params,
            &a2a::SendMessageRequest {
                message,
                configuration: Some(a2a::SendMessageConfiguration {
                    accepted_output_modes: None,
                    task_push_notification_config: None,
                    history_length: None,
                    return_immediately: Some(true),
                }),
                metadata: None,
                tenant: None,
            },
        )
        .await
        .expect("send over HTTP+JSON");
    let a2a::SendMessageResponse::Task(task) = response else {
        panic!("expected a task, got {response:?}");
    };

    let fetched = transport
        .get_task(
            &params,
            &a2a::GetTaskRequest {
                id: task.id.clone(),
                history_length: None,
                tenant: None,
            },
        )
        .await
        .expect("get over HTTP+JSON");
    assert_eq!(fetched.id, task.id);

    let missing = transport
        .get_task(
            &params,
            &a2a::GetTaskRequest {
                id: missing_task_id(&task.id),
                history_length: None,
                tenant: None,
            },
        )
        .await
        .expect_err("an unknown task is an error");
    assert_eq!(missing.code, a2a::error_code::TASK_NOT_FOUND, "{missing:?}");
}
