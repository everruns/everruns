//! Agent Execution API on an `api` channel: agent keys, the per-agent session
//! routes, caller confinement and event visibility.

use crate::test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use test_harness::TestServer;

struct ApiChannel {
    agent_id: String,
    channel_id: String,
}

async fn api_channel(server: &TestServer, config: Value, publish: bool) -> ApiChannel {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("api-agent-{}", uuid::Uuid::new_v4().simple()),
                "display_name": "Support agent",
                "system_prompt": "Test",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent_id = agent["id"].as_str().unwrap().to_string();
    let channel: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/channels"),
            json!({ "channel_type": "api", "channel_config": config }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let channel_id = channel["id"].as_str().unwrap().to_string();
    if publish {
        server
            .post(
                &format!("/v1/agents/{agent_id}/channels/{channel_id}/publish"),
                json!({}),
            )
            .await
            .assert_success();
    }
    ApiChannel {
        agent_id,
        channel_id,
    }
}

async fn create_key(server: &TestServer, channel: &ApiChannel, name: &str) -> Value {
    server
        .post(
            &format!(
                "/v1/agents/{}/channels/{}/keys",
                channel.agent_id, channel.channel_id
            ),
            json!({ "name": name }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

async fn call(
    server: &TestServer,
    method: Method,
    path: &str,
    key: Option<&str>,
    body: Option<Value>,
) -> test_harness::TestResponse {
    let bearer = key.map(|key| format!("Bearer {key}"));
    let mut headers = vec![("content-type", "application/json")];
    if let Some(bearer) = bearer.as_deref() {
        headers.push(("authorization", bearer));
    }
    let body = body
        .map(|body| serde_json::to_vec(&body).unwrap())
        .unwrap_or_default();
    server.request_raw(method, path, headers, body).await
}

fn text_message(text: &str) -> Value {
    json!({ "message": { "role": "user", "content": [{ "type": "text", "text": text }] } })
}

#[tokio::test]
async fn agent_key_runs_a_session_end_to_end() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({}), true).await;
    let key = create_key(&server, &channel, "Support backend").await;
    let secret = key["secret"].as_str().unwrap();
    assert!(secret.starts_with("evr_ak_"));
    assert!(key["id"].as_str().unwrap().starts_with("agentkey_"));
    let base = format!("/v1/channels/{}", channel.channel_id);

    let card: Value = call(&server, Method::GET, &base, Some(secret), None)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(card["name"], channel_agent_name(&server, &channel).await);
    assert_eq!(card["auth"], json!([{ "type": "agent_key" }]));
    assert_eq!(
        card["links"]["sessions"],
        format!("/api/v1/channels/{}/sessions", channel.channel_id)
    );

    // An empty body creates a session; nothing runs until a message arrives.
    let session: Value = call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let session_id = session["id"].as_str().unwrap().to_string();
    // Execution callers never see management detail.
    for hidden in [
        "system_prompt",
        "tools",
        "owner_principal_id",
        "tags",
        "usage",
    ] {
        assert!(session.get(hidden).is_none(), "{hidden} leaked: {session}");
    }

    let message: Value = call(
        &server,
        Method::POST,
        &format!("{base}/sessions/{session_id}/messages"),
        Some(secret),
        Some(text_message("hello agent")),
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    assert_eq!(message["role"], "user");

    let fetched: Value = call(
        &server,
        Method::GET,
        &format!("{base}/sessions/{session_id}"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(fetched["id"], session_id);

    let listed: Value = call(
        &server,
        Method::GET,
        &format!("{base}/sessions"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    let ids: Vec<&str> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert_eq!(ids, vec![session_id.as_str()]);

    let events: Value = call(
        &server,
        Method::GET,
        &format!("{base}/sessions/{session_id}/events"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    let input = events["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "input.message")
        .expect("input.message event");
    assert_eq!(
        input["data"]["message"]["content"][0]["text"],
        "hello agent"
    );
    assert!(input.get("metadata").is_none());

    call(
        &server,
        Method::POST,
        &format!("{base}/sessions/{session_id}/cancel"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::OK);

    // The key's use is recorded.
    let keys: Value = server
        .get(&format!(
            "/v1/agents/{}/channels/{}/keys",
            channel.agent_id, channel.channel_id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(keys[0]["last_used_at"].is_string());
    assert!(keys[0].get("secret").is_none());
}

async fn channel_agent_name(server: &TestServer, channel: &ApiChannel) -> Value {
    let agent: Value = server
        .get(&format!("/v1/agents/{}", channel.agent_id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    agent["display_name"].clone()
}

#[tokio::test]
async fn callers_are_confined_to_their_own_sessions() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({}), true).await;
    let other_channel = api_channel(&server, json!({}), true).await;
    let a = create_key(&server, &channel, "a").await;
    let b = create_key(&server, &channel, "b").await;
    let foreign = create_key(&server, &other_channel, "foreign").await;
    let (a, b, foreign) = (
        a["secret"].as_str().unwrap(),
        b["secret"].as_str().unwrap(),
        foreign["secret"].as_str().unwrap(),
    );
    let base = format!("/v1/channels/{}", channel.channel_id);

    let session: Value = call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        Some(a),
        Some(json!({ "title": "mine" })),
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let session_path = format!("{base}/sessions/{}", session["id"].as_str().unwrap());

    // Another key of the same channel: the session does not exist for it.
    call(&server, Method::GET, &session_path, Some(b), None)
        .await
        .assert_status(StatusCode::NOT_FOUND);
    call(
        &server,
        Method::POST,
        &format!("{session_path}/messages"),
        Some(b),
        Some(text_message("hijack")),
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
    call(
        &server,
        Method::GET,
        &format!("{session_path}/events"),
        Some(b),
        None,
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
    let listed: Value = call(
        &server,
        Method::GET,
        &format!("{base}/sessions"),
        Some(b),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(listed["data"], json!([]));

    // A key of another channel is not a key of this one.
    call(&server, Method::GET, &session_path, Some(foreign), None)
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    // No key, a malformed key and a stranger's PAT-shaped token all fail alike.
    for key in [
        None,
        Some("evr_ak_nope"),
        Some("evr_pat_abcdefabcdefabcdef"),
    ] {
        call(&server, Method::GET, &base, key, None)
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn rotation_keeps_the_old_secret_for_the_overlap_and_revoke_ends_both() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({}), true).await;
    let key = create_key(&server, &channel, "rotating").await;
    let key_id = key["id"].as_str().unwrap();
    let old = key["secret"].as_str().unwrap();
    let keys_path = format!(
        "/v1/agents/{}/channels/{}/keys",
        channel.agent_id, channel.channel_id
    );
    let base = format!("/v1/channels/{}", channel.channel_id);

    server
        .post(
            &format!("{keys_path}/{key_id}/rotate"),
            json!({ "overlap_hours": 169 }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    let rotated: Value = server
        .post(&format!("{keys_path}/{key_id}/rotate"), json!({}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(rotated["id"], key_id);
    assert!(rotated["previous_valid_until"].is_string());
    let new = rotated["secret"].as_str().unwrap();
    assert_ne!(new, old);

    for secret in [old, new] {
        call(&server, Method::GET, &base, Some(secret), None)
            .await
            .assert_status(StatusCode::OK);
    }

    let revoked: Value = server
        .post(&format!("{keys_path}/{key_id}/revoke"), json!({}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(revoked["revoked_at"].is_string());
    for secret in [old, new] {
        call(&server, Method::GET, &base, Some(secret), None)
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }
    server
        .post(&format!("{keys_path}/agentkey_nope/revoke"), json!({}))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unpublished_channels_and_other_channel_types_are_closed() {
    let server = TestServer::in_memory().await;
    let draft = api_channel(&server, json!({}), false).await;
    let key = create_key(&server, &draft, "draft").await;
    call(
        &server,
        Method::GET,
        &format!("/v1/channels/{}", draft.channel_id),
        key["secret"].as_str(),
        None,
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);

    // Keys belong to api channels only.
    let ag_ui: Value = server
        .post(
            &format!("/v1/agents/{}/channels", draft.agent_id),
            json!({ "channel_type": "ag_ui", "channel_config": {} }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let ag_ui_id = ag_ui["id"].as_str().unwrap();
    server
        .post(
            &format!("/v1/agents/{}/channels/{ag_ui_id}/keys", draft.agent_id),
            json!({ "name": "nope" }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
    call(
        &server,
        Method::GET,
        &format!("/v1/channels/{ag_ui_id}"),
        key["secret"].as_str(),
        None,
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);

    // Config is validated: unknown fields and a shared session are refused.
    for config in [
        json!({ "api_key_hash": "x" }),
        json!({ "session_binding": "shared_session" }),
    ] {
        server
            .post(
                &format!("/v1/agents/{}/channels", draft.agent_id),
                json!({ "channel_type": "api", "channel_config": config }),
            )
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn messages_must_be_user_text() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({ "visibility": "messages" }), true).await;
    let key = create_key(&server, &channel, "k").await;
    let secret = key["secret"].as_str();
    let base = format!("/v1/channels/{}", channel.channel_id);
    let session: Value = call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        secret,
        None,
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let messages = format!(
        "{base}/sessions/{}/messages",
        session["id"].as_str().unwrap()
    );
    for body in [
        json!({ "message": { "role": "assistant", "content": [{ "type": "text", "text": "x" }] } }),
        json!({ "message": { "role": "user", "content": [] } }),
        json!({ "message": "plain string" }),
    ] {
        call(&server, Method::POST, &messages, secret, Some(body))
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
    call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        secret,
        Some(json!({ "metadata": { "a": 1 } })),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);
}

/// Starts no turn: a resumed turn stays parked, so a test sees the state the
/// answer left behind.
struct ParkedRunner;

#[async_trait::async_trait]
impl everruns_core::host::TurnBackend for ParkedRunner {
    async fn start_turn(
        &self,
        request: everruns_core::host::TurnRequest,
    ) -> everruns_contracts::error::Result<everruns_core::host::TurnTicket> {
        Ok(everruns_core::host::TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
    ) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: everruns_contracts::typed_id::SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

async fn parked_server() -> TestServer {
    TestServer::in_memory_with_runner(std::sync::Arc::new(ParkedRunner)).await
}

/// A session of `secret`'s caller, parked on `tool_calls` as the engine parks it.
async fn parked_api_session(
    server: &TestServer,
    base: &str,
    secret: &str,
    tool_calls: Value,
) -> String {
    let session: Value = call(
        server,
        Method::POST,
        &format!("{base}/sessions"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let session_id: everruns_contracts::typed_id::SessionId =
        session["id"].as_str().unwrap().parse().unwrap();
    server
        .db
        .update_session(
            1,
            session_id,
            everruns_server::storage::UpdateSession {
                status: Some("waiting_for_tool_results".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update session status")
        .expect("session exists");
    server
        .db
        .create_event(everruns_server::storage::CreateEventRow {
            session_id,
            event_type: "tool.call_requested".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({ "tool_calls": tool_calls }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit tool.call_requested");
    session_id.to_string()
}

fn question_call() -> Value {
    json!({
        "id": "toolu_question",
        "name": "ask_user",
        "arguments": {
            "questions": [{
                "kind": "choice",
                "id": "target",
                "header": "Target",
                "question": "Which environment should I deploy to?",
                "multi_select": false,
                "allow_other": false,
                "options": [
                    {"label": "Staging", "description": "Safe."},
                    {"label": "Production", "description": "Live."}
                ]
            }],
            "timeout_seconds": 300
        }
    })
}

fn approval_call() -> Value {
    let expires = (chrono::Utc::now() + chrono::Duration::minutes(15)).to_rfc3339();
    json!({
        "id": "tool_approval_toolu_mail",
        "name": "approve_tool_call",
        "arguments": {
            "code": "tool_approval_required", "error": "Waiting", "tool_call_id": "toolu_mail",
            "tool": "send_email", "arguments": {"to": "cfo@example.com"},
            "fingerprint": "sha256:ab", "risk": "open_world", "mode": "normal",
            "asked_at": chrono::Utc::now().to_rfc3339(), "expires_at": expires
        }
    })
}

#[tokio::test]
async fn the_caller_answers_questions_and_sees_only_that_an_approval_waits() {
    let server = parked_server().await;
    let channel = api_channel(&server, json!({ "visibility": "messages" }), true).await;
    let key = create_key(&server, &channel, "k").await;
    let other = create_key(&server, &channel, "other").await;
    let (secret, other) = (
        key["secret"].as_str().unwrap(),
        other["secret"].as_str().unwrap(),
    );
    let base = format!("/v1/channels/{}", channel.channel_id);
    let internal =
        json!({ "id": "toolu_lookup", "name": "internal_lookup", "arguments": { "q": "s3cr3t" } });
    let session_id = parked_api_session(
        &server,
        &base,
        secret,
        json!([question_call(), approval_call(), internal]),
    )
    .await;
    let session_path = format!("{base}/sessions/{session_id}");

    let session: Value = call(&server, Method::GET, &session_path, Some(secret), None)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        session["pending_questions"][0]["tool_call_id"],
        "toolu_question"
    );
    assert_eq!(
        session["pending_questions"][0]["questions"][0]["id"],
        "target"
    );
    // An operator decides by default: the caller sees that a call waits, not
    // which tool or with what.
    let approval = &session["pending_approvals"][0];
    assert_eq!(approval["tool_call_id"], "tool_approval_toolu_mail");
    assert_eq!(approval["answerable"], false);
    for hidden in ["send_email", "cfo@example.com", "s3cr3t", "internal_lookup"] {
        assert!(
            !session.to_string().contains(hidden),
            "{hidden} leaked: {session}"
        );
    }
    let events: Value = call(
        &server,
        Method::GET,
        &format!("{session_path}/events"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    let requested = events["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "tool.call_requested")
        .expect("the question reaches the event list");
    assert_eq!(
        requested["data"]["pending_questions"][0]["tool_call_id"],
        "toolu_question"
    );
    assert!(!requested.to_string().contains("s3cr3t"));

    call(
        &server,
        Method::POST,
        &format!("{session_path}/tool-approvals"),
        Some(secret),
        Some(json!({ "decisions": [{ "tool_call_id": "tool_approval_toolu_mail", "decision": "allow" }] })),
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);
    // A credential never travels over the execution API.
    call(
        &server,
        Method::POST,
        &format!("{session_path}/question-answers"),
        Some(secret),
        Some(json!({ "answers": [{ "id": "target", "secret_ref": "session:STRIPE_API_KEY" }] })),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);
    // Another caller cannot answer for this one.
    let answer = json!({ "tool_call_id": "toolu_question", "answers": [{ "id": "target", "selected": ["Staging"] }] });
    call(
        &server,
        Method::POST,
        &format!("{session_path}/question-answers"),
        Some(other),
        Some(answer.clone()),
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
    // Answers are checked against what was asked.
    call(
        &server,
        Method::POST,
        &format!("{session_path}/question-answers"),
        Some(secret),
        Some(json!({ "answers": [{ "id": "target", "selected": ["Moon"] }] })),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);

    let answered: Value = call(
        &server,
        Method::POST,
        &format!("{session_path}/question-answers"),
        Some(secret),
        Some(answer.clone()),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(answered["status"], "answered");
    call(
        &server,
        Method::POST,
        &format!("{session_path}/question-answers"),
        Some(secret),
        Some(answer),
    )
    .await
    .assert_status(StatusCode::CONFLICT);
}

#[tokio::test]
async fn the_caller_approves_tools_when_the_channel_lets_it() {
    let server = parked_server().await;
    let channel = api_channel(&server, json!({ "tool_approvals": "caller" }), true).await;
    let key = create_key(&server, &channel, "k").await;
    let secret = key["secret"].as_str().unwrap();
    let base = format!("/v1/channels/{}", channel.channel_id);
    let session_id = parked_api_session(&server, &base, secret, json!([approval_call()])).await;
    let session_path = format!("{base}/sessions/{session_id}");

    let session: Value = call(&server, Method::GET, &session_path, Some(secret), None)
        .await
        .assert_status(StatusCode::OK)
        .json();
    let approval = &session["pending_approvals"][0];
    assert_eq!(approval["answerable"], true);
    assert_eq!(approval["tool_name"], "send_email");
    assert_eq!(approval["arguments"]["to"], "cfo@example.com");

    call(
        &server,
        Method::POST,
        &format!("{session_path}/tool-approvals"),
        Some(secret),
        Some(
            json!({ "decisions": [{ "tool_call_id": "tool_approval_nope", "decision": "allow" }] }),
        ),
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
    let resolved: Value = call(
        &server,
        Method::POST,
        &format!("{session_path}/tool-approvals"),
        Some(secret),
        Some(json!({ "decisions": [{ "tool_call_id": "tool_approval_toolu_mail", "decision": "allow" }] })),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(resolved["resolved"][0]["outcome"], "allow");
}

#[tokio::test]
async fn the_event_stream_applies_visibility() {
    let server = parked_server().await;
    let channel = api_channel(&server, json!({ "visibility": "messages" }), true).await;
    let key = create_key(&server, &channel, "k").await;
    let other = create_key(&server, &channel, "other").await;
    let secret = key["secret"].as_str().unwrap();
    let base = format!("/v1/channels/{}", channel.channel_id);
    let card: Value = call(&server, Method::GET, &base, Some(secret), None)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(card["streaming"], true);

    let session: Value = call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let session_id = session["id"].as_str().unwrap();
    call(
        &server,
        Method::POST,
        &format!("{base}/sessions/{session_id}/messages"),
        Some(secret),
        Some(text_message("hello stream")),
    )
    .await
    .assert_status(StatusCode::CREATED);
    server
        .db
        .create_event(everruns_server::storage::CreateEventRow {
            session_id: session_id.parse().unwrap(),
            event_type: "tool.started".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({ "tool_call": { "id": "c1", "name": "secret_tool", "arguments": {} } }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit tool.started");

    let sse = format!("{base}/sessions/{session_id}/sse?after_sequence=0");
    let bearer = format!("Bearer {secret}");
    let stream = server
        .get_stream_prefix_with_headers(
            &sse,
            &[("authorization", bearer.as_str())],
            64 * 1024,
            std::time::Duration::from_secs(2),
        )
        .await;
    assert!(stream.contains("event: connected"), "{stream}");
    assert!(stream.contains("event: input.message"), "{stream}");
    assert!(stream.contains("hello stream"), "{stream}");
    assert!(!stream.contains("secret_tool"), "{stream}");

    // The stream is confined like every other route.
    for key in [None, Some(other["secret"].as_str().unwrap())] {
        let status = if key.is_some() {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::UNAUTHORIZED
        };
        call(&server, Method::GET, &sse, key, None)
            .await
            .assert_status(status);
    }
}
