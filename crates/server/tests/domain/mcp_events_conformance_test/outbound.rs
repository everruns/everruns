use super::*;
use std::path::PathBuf;

/// Production URL validation and signing run before this test-only egress
/// maps a synthetic globally-routable IP to the official loopback receiver.
struct ReceiverEgress(PathBuf);
#[async_trait::async_trait]
impl EgressService for ReceiverEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let parsed = url::Url::parse(&request.url).unwrap();
        assert_eq!(parsed.scheme(), "https");
        assert_eq!(parsed.host_str(), Some("8.8.8.8"));
        let port: u16 = std::fs::read_to_string(&self.0).unwrap().parse().unwrap();
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .post(format!("http://127.0.0.1:{port}{}", parsed.path()))
            .timeout(std::time::Duration::from_secs(5))
            .body(request.body);
        for (key, value) in request.headers {
            builder = builder.header(key, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|e| everruns_core::EgressError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        // Fault injection makes retry timestamps observably different seconds.
        if status == 503 {
            tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        }
        Ok(EgressResponse {
            status,
            headers: Default::default(),
            body: response.bytes().await.unwrap().to_vec(),
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

async fn tool(base: &str, name: &str, arguments: Value) -> Value {
    let response: Value = reqwest::Client::new().post(format!("{base}/mcp"))
        .header("MCP-Protocol-Version", "2026-07-28").header("Mcp-Name", name)
        .json(&json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":name,"arguments":arguments}}))
        .send().await.unwrap().json().await.unwrap();
    assert!(response.get("error").is_none(), "{response}");
    assert_ne!(response["result"]["isError"], true, "{response}");
    response["result"].clone()
}

pub(super) async fn fixture(
    port_file: PathBuf,
) -> (TestServer, String, tokio::task::JoinHandle<()>) {
    let server = TestServer::in_memory_authenticated().await;
    let subscriber = crate::mcp_events_test::member(&server).await;
    let generated = everruns_server::auth::personal_access_token::generate_personal_access_token();
    server
        .db
        .create_personal_access_token(everruns_server::storage::CreatePersonalAccessTokenRow {
            user_id: subscriber.user_id,
            name: "conformance-only".into(),
            token_hash: generated.token_hash,
            token_prefix: generated.token_prefix,
            scopes: vec!["*".into()],
            expires_at: None,
            metadata: json!({}),
        })
        .await
        .unwrap();
    let base = server.serve_mcp_conformance(&generated.token).await;
    let created = tool(&base, "execute", json!({"commands":"create_agent --name conformance-events --display_name Events --system_prompt unused"})).await;
    let agent: Value =
        serde_json::from_str(created["content"][0]["text"].as_str().unwrap()).unwrap();
    let run = tool(
        &base,
        "agent_run",
        json!({"agent_id":agent["id"],"message":"test"}),
    )
    .await;
    let session = run["structuredContent"]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    *server.webhooks.forward.write() = Some(Arc::new(ReceiverEgress(port_file)));
    let service = server.mcp_events.clone();
    let pump = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            service
                .dispatch(&crate::mcp_events_test::turn_completed(&session))
                .await;
        }
    });
    (server, base, pump)
}
