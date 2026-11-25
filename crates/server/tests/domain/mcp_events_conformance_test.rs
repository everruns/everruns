//! Official upstream checks over /mcp and the production inbound trigger client.
//! Opt in with MCP_EVENTS_CONFORMANCE_DIR; all fixtures are synthetic/local.
use crate::test_harness::TestServer;
use everruns_core::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

#[path = "mcp_events_conformance_test/outbound.rs"]
mod outbound;

const REMOTE: &str = "https://8.8.8.8/mcp";

/// Only this test adapter maps a DNS-pinned synthetic public URL to the
/// runner's loopback mock. Production SSRF guards remain in the call path.
struct RunnerEgress(String);
#[async_trait::async_trait]
impl EgressService for RunnerEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        assert_eq!(request.url, REMOTE);
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .post(&self.0)
            .body(request.body);
        for (key, value) in request.headers {
            builder = builder.header(key, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|e| everruns_core::EgressError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response.bytes().await.unwrap().to_vec();
        assert!(body.len() < 256 * 1024);
        Ok(EgressResponse {
            status,
            headers: Default::default(),
            body,
        })
    }
    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        let response = self.send(request).await?;
        Ok(EgressStreamResponse {
            status: response.status,
            headers: response.headers,
            body: Box::pin(futures::stream::iter([Ok(response.body)])),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_driver() {
    let Ok(url) = std::env::var("EVERRUNS_EVENTS_URL") else {
        return;
    };
    let mut parsed = url::Url::parse(&url).unwrap();
    assert!(matches!(parsed.host_str(), Some("localhost" | "127.0.0.1")));
    assert_eq!(parsed.scheme(), "http");
    parsed.set_host(Some("127.0.0.1")).unwrap();
    let egress = Arc::new(RunnerEgress(parsed.to_string()));
    let server = TestServer::in_memory().await;
    *server.mcp_servers.0.write() = Some(egress.clone());
    let list = everruns_core::mcp::http_request(
        &*egress,
        REMOTE,
        &HashMap::new(),
        None,
        "events/list",
        &json!({}),
    )
    .await;
    if std::env::var("MCP_CONFORMANCE_SCENARIO").unwrap() == "events-client-no-extension" {
        assert!(list.is_err());
        return;
    }
    let list = list.unwrap();
    // Production starts the refresher before any trigger exists.
    let refresher = server.mcp_event_triggers.spawn_refresher();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let agent: Value = server.post("/v1/platform/agents", json!({
        "name":"events-conformance", "system_prompt":"unused", "harness_id":server.seed_generic_harness_id,
        "mcpServers":{"events":{"url":REMOTE}}
    })).await.assert_status(axum::http::StatusCode::CREATED).json();
    for descriptor in list["events"].as_array().unwrap() {
        assert_eq!(descriptor["delivery"], json!(["webhook"]));
        server.post(&format!("/v1/platform/agents/{}/triggers", agent["id"].as_str().unwrap()), json!({
            "trigger_type":"mcp_event", "mcp_server":"events", "mcp_event":descriptor["name"],
            "mcp_event_arguments":{}, "session_mode":"session_per_invocation", "message":"test", "enabled":true
        })).await.assert_status(axum::http::StatusCode::CREATED);
    }
    let context: Value =
        serde_json::from_str(&std::env::var("MCP_CONFORMANCE_CONTEXT").unwrap()).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(
        context["durationMs"].as_u64().unwrap(),
    ))
    .await;
    refresher.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn official_events_checks() {
    let Ok(dir) = std::env::var("MCP_EVENTS_CONFORMANCE_DIR") else {
        eprintln!("Official Events checks require the dedicated conformance workflow/setup.");
        return;
    };
    let receiver = tempfile::tempdir().unwrap();
    let receiver_port = receiver.path().join("port");
    let (_server, base, pump) = outbound::fixture(receiver_port.clone()).await;
    let output = tokio::process::Command::new("node")
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .args([
            "scripts/mcp-events-conformance/run.mjs",
            &dir,
            &base,
            std::env::current_exe().unwrap().to_str().unwrap(),
            receiver_port.to_str().unwrap(),
        ])
        // The reviewed but unreleased runner never receives developer secrets.
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env(
            "LD_LIBRARY_PATH",
            std::env::var("LD_LIBRARY_PATH").unwrap_or_default(),
        )
        .env("TMPDIR", "/tmp")
        .env("HOME", "/tmp")
        .env(
            "EVERRUNS_PG_CACHE",
            std::env::var("EVERRUNS_PG_CACHE").unwrap_or_else(|_| {
                std::env::var("XDG_CACHE_HOME")
                    .map(|p| format!("{p}/everruns/postgres"))
                    .unwrap_or_else(|_| {
                        format!(
                            "{}/.cache/everruns/postgres",
                            std::env::var("HOME").unwrap()
                        )
                    })
            }),
        )
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    pump.abort();
    println!("{}", String::from_utf8_lossy(&output.stdout));
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "official Events checks failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn official_mcp_baseline() {
    let Ok(dir) = std::env::var("MCP_CONFORMANCE_DIR") else {
        eprintln!("Released MCP baseline requires its pinned runner installation.");
        return;
    };
    let (_server, base) = TestServer::serving_in_memory().await;
    let output = tokio::process::Command::new("node")
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .args([
            &format!("{dir}/dist/index.js"),
            "server",
            "--url",
            &format!("{base}/mcp"),
            "--scenario",
            "server-initialize",
        ])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    println!("{}", String::from_utf8_lossy(&output.stdout));
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "released MCP baseline failed");
}
