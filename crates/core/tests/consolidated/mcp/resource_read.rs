//! Resource MRTR must retain the original read and stop on refusal or cancellation.
use async_trait::async_trait;
use everruns_core::mcp::{
    ElicitationAction, McpClient, McpConnection, StaticAuthProvider, UrlElicitation,
    UrlElicitationHandler,
};
use everruns_core::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const URL: &str = "http://8.8.8.8/mcp";
const URI: &str = "docs://private/a%2Fb?revision=3";
const STATE: &str = " opaque\nstate/+/=☃ ";

struct ScriptedEgress {
    results: Mutex<VecDeque<Value>>,
    requests: Mutex<Vec<EgressRequest>>,
    sse: bool,
    delay: std::time::Duration,
}

impl ScriptedEgress {
    fn new(results: Vec<Value>) -> Arc<Self> {
        Arc::new(Self {
            results: Mutex::new(results.into()),
            requests: Mutex::new(vec![]),
            sse: false,
            delay: std::time::Duration::ZERO,
        })
    }
    fn bodies(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect()
    }
}

#[async_trait]
impl EgressService for ScriptedEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        self.requests.lock().unwrap().push(request);
        tokio::time::sleep(self.delay).await;
        let mut headers = std::collections::BTreeMap::new();
        let result = match body["method"].as_str().unwrap() {
            "initialize" => {
                headers.insert("Mcp-Session-Id".into(), "original-session".into());
                json!({"protocolVersion":"2025-06-18", "capabilities":{}, "serverInfo":{"name":"docs","version":"1"}})
            }
            "notifications/initialized" => {
                return Ok(EgressResponse {
                    status: 202,
                    headers,
                    body: vec![],
                });
            }
            _ => self
                .results
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected request"),
        };
        let response = if result.get("error").is_some() {
            json!({"jsonrpc":"2.0", "id":body["id"], "error":result["error"]})
        } else {
            json!({"jsonrpc":"2.0", "id":body["id"], "result":result})
        };
        let body = if self.sse {
            format!("event: message\ndata: {response}\n\n").into_bytes()
        } else {
            serde_json::to_vec(&response).unwrap()
        };
        Ok(EgressResponse {
            status: 200,
            headers,
            body,
        })
    }
    async fn send_stream(&self, _: EgressRequest) -> EgressResult<EgressStreamResponse> {
        unreachable!()
    }
}

struct Consent {
    action: ElicitationAction,
    delay: std::time::Duration,
    seen: Mutex<Vec<UrlElicitation>>,
}

impl Consent {
    fn new(action: ElicitationAction) -> Arc<Self> {
        Arc::new(Self {
            action,
            delay: std::time::Duration::ZERO,
            seen: Mutex::new(vec![]),
        })
    }
}

#[async_trait]
impl UrlElicitationHandler for Consent {
    async fn request_url_consent(
        &self,
        request: &UrlElicitation,
    ) -> anyhow::Result<ElicitationAction> {
        self.seen.lock().unwrap().push(request.clone());
        tokio::time::sleep(self.delay).await;
        Ok(self.action)
    }
}

fn client(egress: Arc<ScriptedEgress>, consent: Option<Arc<Consent>>) -> McpClient {
    let auth = Arc::new(StaticAuthProvider::new().with_bearer("docs", "original-identity"));
    match consent {
        Some(consent) => McpClient::with_url_elicitation(egress, auth, consent),
        None => McpClient::new(egress, auth),
    }
}

fn complete() -> Value {
    json!({"resultType":"complete", "contents":[{"uri":URI,"mimeType":"text/plain","text":"private content"}]})
}

fn elicit() -> Value {
    json!({"resultType":"input_required", "requestState":STATE, "inputRequests":{
        "consent":{"method":"elicitation/create", "params":{"mode":"url", "url":"https://consent.example/authorize", "message":"Authorize this read"}}
    }})
}

#[tokio::test]
async fn resource_read_retries_opaque_state_without_changing_uri_or_identity() {
    let egress = ScriptedEgress::new(vec![
        json!({"resultType":"input_required", "requestState":STATE}),
        complete(),
    ]);
    let result = client(egress.clone(), None)
        .read_resource(&McpConnection::http("docs", URL), URI)
        .await
        .unwrap();
    assert_eq!(result.contents.len(), 1);
    let bodies = egress.bodies();
    assert_eq!(bodies.len(), 2);
    for body in &bodies {
        assert_eq!(body["method"], "resources/read");
        assert_eq!(body["params"]["uri"], URI);
        assert!(body["params"].get("name").is_none());
        assert_eq!(
            body["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"],
            json!({})
        );
    }
    assert_eq!(bodies[1]["params"]["requestState"], STATE);
    assert_ne!(bodies[0]["id"], bodies[1]["id"]);
    for request in egress.requests.lock().unwrap().iter() {
        assert_eq!(request.url, URL);
        assert_eq!(request.headers["Authorization"], "Bearer original-identity");
        assert_eq!(request.headers["Mcp-Method"], "resources/read");
        assert!(!request.headers.contains_key("Mcp-Name"));
    }
}

#[tokio::test]
async fn accepted_resource_consent_is_scoped_to_method_and_exact_uri() {
    let egress = ScriptedEgress::new(vec![elicit(), complete(), elicit(), complete()]);
    let consent = Consent::new(ElicitationAction::Accept);
    let client = client(egress.clone(), Some(consent.clone()));
    let connection = McpConnection::http("docs", URL);
    client.read_resource(&connection, URI).await.unwrap();
    client
        .read_resource(&connection, "docs://different")
        .await
        .unwrap();
    let seen = consent.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].server_name, "docs");
    assert_eq!(seen[0].tool_name, format!("resources/read:{URI}"));
    assert_eq!(seen[1].tool_name, "resources/read:docs://different");
    assert_eq!(seen[0].host, "consent.example");
    let bodies = egress.bodies();
    for i in [0, 2] {
        assert!(bodies[i]["params"].get("requestState").is_none());
        assert_eq!(
            bodies[i]["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"],
            json!({"elicitation":{"url":{}}})
        );
        assert_eq!(
            bodies[i + 1]["params"]["inputResponses"],
            json!({"consent":{"action":"accept"}})
        );
    }
}

#[tokio::test]
async fn refusal_or_cancellation_never_retries_a_resource() {
    for action in [ElicitationAction::Decline, ElicitationAction::Cancel] {
        let egress = ScriptedEgress::new(vec![elicit()]);
        let error = client(egress.clone(), Some(Consent::new(action)))
            .read_resource(&McpConnection::http("docs", URL), URI)
            .await
            .unwrap_err();
        let pending = error
            .downcast_ref::<everruns_core::mcp::UrlElicitationPending>()
            .unwrap();
        assert_eq!(pending.action, action);
        assert_eq!(pending.tool_name, format!("resources/read:{URI}"));
        assert_eq!(egress.bodies().len(), 1);
    }
}

#[tokio::test]
async fn missing_handler_and_disabled_policy_never_claim_consent() {
    for configured in [false, true] {
        let egress = ScriptedEgress::new(vec![elicit()]);
        let consent = Consent::new(ElicitationAction::Accept);
        let client = client(egress.clone(), configured.then(|| consent.clone()));
        let mut connection = McpConnection::http("docs", URL);
        if configured {
            connection.elicitation_policy = everruns_core::McpElicitationPolicy::None;
        }
        let error = client.read_resource(&connection, URI).await.unwrap_err();
        assert!(
            error.to_string().contains("no elicitation capability"),
            "{error}"
        );
        assert!(consent.seen.lock().unwrap().is_empty());
        let bodies = egress.bodies();
        assert_eq!(bodies.len(), 1);
        assert_eq!(
            bodies[0]["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"],
            json!({})
        );
    }
}

#[tokio::test]
async fn resource_read_rejects_unsupported_input_and_unsafe_urls() {
    for (request, message) in [
        (
            json!({"method":"sampling/createMessage", "params":{}}),
            "does not support",
        ),
        (
            json!({"method":"roots/list", "params":{}}),
            "does not support",
        ),
        (
            json!({"method":"elicitation/create", "params":{"mode":"url","url":"javascript:alert(1)","message":"open"}}),
            "unusable elicitation URL",
        ),
    ] {
        let mut result = elicit();
        result["inputRequests"]["consent"] = request;
        let egress = ScriptedEgress::new(vec![result]);
        let consent = Consent::new(ElicitationAction::Accept);
        let error = client(egress.clone(), Some(consent.clone()))
            .read_resource(&McpConnection::http("docs", URL), URI)
            .await
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
        assert!(consent.seen.lock().unwrap().is_empty());
        assert_eq!(egress.bodies().len(), 1);
    }
}

#[tokio::test]
async fn resource_rounds_replace_state_and_responses_and_stop_at_the_limit() {
    let egress = ScriptedEgress::new(vec![
        elicit(),
        json!({"resultType":"input_required","requestState":"replacement"}),
        complete(),
    ]);
    client(
        egress.clone(),
        Some(Consent::new(ElicitationAction::Accept)),
    )
    .read_resource(&McpConnection::http("docs", URL), URI)
    .await
    .unwrap();
    let bodies = egress.bodies();
    assert_eq!(bodies[2]["params"]["requestState"], "replacement");
    assert!(bodies[2]["params"].get("inputResponses").is_none());
    assert_ne!(bodies[1]["id"], bodies[2]["id"]);

    let mut no_state = elicit();
    no_state.as_object_mut().unwrap().remove("requestState");
    let egress = ScriptedEgress::new(vec![elicit(), no_state, elicit()]);
    let error = client(
        egress.clone(),
        Some(Consent::new(ElicitationAction::Accept)),
    )
    .read_resource(&McpConnection::http("docs", URL), URI)
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("still requires input after 2 rounds"),
        "{error}"
    );
    let bodies = egress.bodies();
    assert_eq!(bodies.len(), 3);
    assert!(bodies[2]["params"].get("requestState").is_none());
}

#[tokio::test(start_paused = true)]
async fn resource_deadline_covers_all_consent_rounds() {
    let egress = ScriptedEgress::new(vec![elicit(), elicit(), complete()]);
    let consent = Arc::new(Consent {
        action: ElicitationAction::Accept,
        delay: std::time::Duration::from_secs(40),
        seen: Mutex::new(vec![]),
    });
    let started = tokio::time::Instant::now();
    let error = client(egress.clone(), Some(consent.clone()))
        .read_resource(&McpConnection::http("docs", URL), URI)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("resources/read timed out"),
        "{error}"
    );
    assert_eq!(started.elapsed(), std::time::Duration::from_secs(60));
    assert_eq!(consent.seen.lock().unwrap().len(), 2);
    assert_eq!(
        egress.bodies().len(),
        2,
        "second consent wait must not restart the deadline"
    );
}

#[tokio::test(start_paused = true)]
async fn dropping_resource_read_cancels_consent_without_background_retries() {
    let egress = ScriptedEgress::new(vec![elicit(), complete()]);
    let consent = Arc::new(Consent {
        action: ElicitationAction::Accept,
        delay: std::time::Duration::from_secs(40),
        seen: Mutex::new(vec![]),
    });
    let client = client(egress.clone(), Some(consent.clone()));
    let connection = McpConnection::http("docs", URL);
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            client.read_resource(&connection, URI)
        )
        .await
        .is_err()
    );
    tokio::time::advance(std::time::Duration::from_secs(100)).await;
    assert_eq!(consent.seen.lock().unwrap().len(), 1);
    assert_eq!(egress.bodies().len(), 1);
}

#[tokio::test]
async fn resource_discovery_does_not_advertise_or_answer_inputs() {
    let egress = ScriptedEgress::new(vec![
        json!({"resources":[]}),
        json!({"resourceTemplates":[]}),
    ]);
    let consent = Consent::new(ElicitationAction::Accept);
    let client = client(egress.clone(), Some(consent.clone()));
    let connection = McpConnection::http("docs", URL);
    client.list_resources(&connection, None).await.unwrap();
    client
        .list_resource_templates(&connection, None)
        .await
        .unwrap();
    for body in egress.bodies() {
        assert_eq!(
            body["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"],
            json!({})
        );
    }
    assert!(consent.seen.lock().unwrap().is_empty());
}

struct FormConsent(Mutex<Vec<everruns_core::mcp::FormElicitation>>);

#[async_trait]
impl everruns_core::mcp::FormElicitationHandler for FormConsent {
    async fn answer_form(
        &self,
        request: &everruns_core::mcp::FormElicitation,
    ) -> anyhow::Result<everruns_core::mcp::FormOutcome> {
        self.0.lock().unwrap().push(request.clone());
        Ok(everruns_core::mcp::FormOutcome::Accept(
            json!({"language":"uk"}).as_object().unwrap().clone(),
        ))
    }
}

#[tokio::test]
async fn resource_forms_require_opt_in_and_preserve_typed_answers() {
    for allowed in [false, true] {
        let result = json!({"resultType":"input_required", "requestState":STATE,"inputRequests":{
            "language":{"method":"elicitation/create", "params":{"mode":"form","message":"Choose language", "requestedSchema":{"type":"object","properties":{"language":{"type":"string","enum":["en","uk"]}},"required":["language"]}}}
        }});
        let egress = ScriptedEgress::new(vec![result, complete()]);
        let form = Arc::new(FormConsent(Mutex::new(vec![])));
        let client = McpClient::with_url_and_form_elicitation(
            egress.clone(),
            Arc::new(StaticAuthProvider::new()),
            Consent::new(ElicitationAction::Accept),
            form.clone(),
        );
        let mut connection = McpConnection::http("docs", URL);
        if allowed {
            connection.elicitation_policy = everruns_core::McpElicitationPolicy::UrlAndForm;
        }
        let result = client.read_resource(&connection, URI).await;
        if allowed {
            result.unwrap();
            assert_eq!(
                form.0.lock().unwrap()[0].tool_name,
                format!("resources/read:{URI}")
            );
            let bodies = egress.bodies();
            assert_eq!(
                bodies[1]["params"]["inputResponses"]["language"],
                json!({"action":"accept","content":{"language":"uk"}})
            );
            assert_eq!(
                bodies[0]["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"]["elicitation"],
                json!({"url":{},"form":{}})
            );
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("did not declare form mode")
            );
            assert!(form.0.lock().unwrap().is_empty());
            assert_eq!(egress.bodies().len(), 1);
        }
    }
}

#[tokio::test]
async fn resource_retry_preserves_negotiation_and_parses_sse() {
    let mut egress = ScriptedEgress::new(vec![
        json!({"resultType":"input_required","requestState":STATE}),
        complete(),
    ]);
    Arc::get_mut(&mut egress).unwrap().sse = true;
    let connection = McpConnection::http("docs", URL)
        .with_protocol_mode(everruns_core::McpProtocolMode::V2025June);
    client(egress.clone(), None)
        .read_resource(&connection, URI)
        .await
        .unwrap();
    let requests = egress.requests.lock().unwrap();
    let reads: Vec<_> = requests
        .iter()
        .filter(|r| {
            r.headers
                .get("Mcp-Method")
                .is_some_and(|m| m == "resources/read")
        })
        .collect();
    assert_eq!(reads.len(), 2);
    for read in reads {
        assert_eq!(read.headers["Mcp-Session-Id"], "original-session");
        assert_eq!(read.headers["MCP-Protocol-Version"], "2025-06-18");
        assert_eq!(read.headers["Authorization"], "Bearer original-identity");
    }
}

#[tokio::test]
async fn resource_retry_propagates_rpc_errors() {
    let egress = ScriptedEgress::new(vec![
        json!({"resultType":"input_required","requestState":STATE}),
        json!({"error":{"code":-32002,"message":"Resource not found","data":{"uri":URI}}}),
    ]);
    let error = client(egress.clone(), None)
        .read_resource(&McpConnection::http("docs", URL), URI)
        .await
        .unwrap_err();
    let error = error
        .downcast_ref::<everruns_core::mcp::McpRpcError>()
        .unwrap();
    assert_eq!(error.code, -32002);
    assert_eq!(error.data.as_ref().unwrap()["uri"], URI);
    assert_eq!(egress.bodies().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn resource_deadline_includes_initial_network_request() {
    let mut egress = ScriptedEgress::new(vec![elicit(), complete()]);
    Arc::get_mut(&mut egress).unwrap().delay = std::time::Duration::from_secs(40);
    let consent = Arc::new(Consent {
        action: ElicitationAction::Accept,
        delay: std::time::Duration::from_secs(30),
        seen: Mutex::new(vec![]),
    });
    let start = tokio::time::Instant::now();
    let error = client(egress.clone(), Some(consent))
        .read_resource(&McpConnection::http("docs", URL), URI)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert_eq!(start.elapsed(), std::time::Duration::from_secs(60));
    assert_eq!(egress.bodies().len(), 1);
}

#[tokio::test]
async fn free_resource_request_handles_state_only_continuations() {
    let egress = ScriptedEgress::new(vec![
        json!({"resultType":"input_required", "requestState":STATE}),
        complete(),
    ]);
    let result = everruns_core::mcp::http_request(
        egress.as_ref(),
        URL,
        &Default::default(),
        None,
        "resources/read",
        &json!({"uri":URI}),
    )
    .await
    .unwrap();
    assert_eq!(result, complete());
    assert_eq!(egress.bodies()[1]["params"]["uri"], URI);
}

#[tokio::test]
async fn tool_cannot_consume_a_resource_consent_scope() {
    let egress = ScriptedEgress::new(vec![elicit()]);
    let consent = Consent::new(ElicitationAction::Accept);
    let error = client(egress.clone(), Some(consent.clone()))
        .call(
            &McpConnection::http("docs", URL),
            &format!("resources/read:{URI}"),
            json!({}),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("reserved resource consent scope")
    );
    assert!(consent.seen.lock().unwrap().is_empty());
    assert_eq!(egress.bodies().len(), 1);
}
