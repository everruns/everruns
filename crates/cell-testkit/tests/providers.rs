//! An org's own model provider on the data plane (data plane v2, step 8):
//! the control plane sends it as an `llm_provider` connection whose key is
//! a sealed secret, and only the executor that runs the turn opens it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::sync::Arc;

use axum::Router;
use axum::http::HeaderMap;
use axum::routing::post;
use everruns_cell::dp::{ConfigAck, ConfigChange, ConfigUpdate};
use everruns_cell::node_control::ControlPlane;
use everruns_cell_testkit::fake_cp::FakeCp;
use everruns_contracts::dataplane::Connection;
use everruns_contracts::dataplane_seal as seal;
use parking_lot::Mutex;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use support::{Fleet, answers};

const ORG: &str = "org-a";

/// What the fake model API saw: (authorization, model) per call.
type Seen = Arc<Mutex<Vec<(String, String)>>>;

/// An OpenAI-compatible chat API that answers "pong".
async fn model_api() -> (String, Seen) {
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |headers: HeaderMap, body: axum::Json<Value>| {
            let log = log.clone();
            async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                let model = body["model"].as_str().unwrap_or_default().to_owned();
                log.lock().push((auth, model));
                let sse = [
                    r#"data: {"id":"1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"pong"},"finish_reason":null}]}"#,
                    "",
                    r#"data: {"id":"1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#,
                    "",
                    "data: [DONE]",
                    "",
                    "",
                ]
                .join("\n");
                (
                    [
                        ("content-type", "text/event-stream"),
                        ("connection", "close"),
                    ],
                    sse,
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    (url, seen)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_calls_the_orgs_own_provider_with_its_sealed_key() {
    let (model_url, seen) = model_api().await;
    let cp = FakeCp::start().await.unwrap();
    cp.set_balance(ORG, 10_000_000);
    let mut fleet = Fleet::start_with_cp(Some(ControlPlane::new(cp.url.clone()))).await;
    let base = fleet.base();

    // The org, with its data key wrapped under the data plane's key.
    let dp_key = seal::random_key();
    let data_key = seal::random_key();
    let wrapped =
        seal::wrap_data_key(&dp_key, ORG, "dk-1", &data_key, seal::random_nonce()).unwrap();
    let (status, ack) = cp
        .call(
            &base,
            Method::PUT,
            &format!("/dp/v1/orgs/{ORG}"),
            &json!({ "command_id": "enable-1", "wrapped_data_key": wrapped, "data_key_id": "dk-1" }),
        )
        .await
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{ack}");

    // Its provider "Acme": an OpenAI-compatible API, and its key.
    let connection = everruns_cell::providers::connection_id("Acme");
    let key = seal::seal(
        &data_key,
        "dk-1",
        ORG,
        &connection,
        1,
        b"key-of-org-a",
        seal::random_nonce(),
    )
    .unwrap();
    let settings = everruns_cell::providers::Settings {
        provider_key: "acme".into(),
        provider_type: "openai".into(),
        base_url: Some(model_url),
        api: Some("chat_completions".into()),
        request_options: Default::default(),
    };
    let mut package = everruns_cell_testkit::definition("echo").unwrap();
    package["name"] = json!("helper");
    package["model"] = json!({ "provider": "Acme", "model": "acme-large" });
    let ack = cp
        .push_config(
            &base,
            ORG,
            &ConfigUpdate {
                epoch: 1,
                version: 1,
                base_version: 0,
                changes: vec![
                    ConfigChange::Secret(key).to_value(),
                    ConfigChange::Connection(Connection {
                        connection_id: connection.clone(),
                        kind: everruns_cell::providers::KIND.into(),
                        settings: serde_json::to_value(&settings).unwrap(),
                        token_secret_id: connection.clone(),
                        refresh_owner: Default::default(),
                    })
                    .to_value(),
                    json!({ "kind": "Agent", "agent_id": "helper", "revision": 1, "package": package }),
                ],
            },
        )
        .await
        .unwrap();
    assert_eq!(ack, ConfigAck::Ok { applied_version: 1 });
    let executor = fleet.executor(1);
    executor.set_dp_key(dp_key);

    let created = fleet
        .post("/v1/sessions", json!({ "org": ORG, "agent_ref": "helper" }))
        .await;
    let session = created["id"].as_str().unwrap().to_owned();
    fleet.say(&session, "ping").await;
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    assert_eq!(answers(&events), vec!["pong".to_string()], "{events:?}");
    let calls = seen.lock().clone();
    assert!(!calls.is_empty());
    for (auth, model) in calls {
        assert_eq!(auth, "Bearer key-of-org-a");
        assert_eq!(model, "acme-large");
    }
}
