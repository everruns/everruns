//! URL mode elicitation integration tests (knowledge/integrations/mcp-servers.md
//! "URL mode elicitation").
//!
//! A scripted [`EgressService`] plays a `2026-07-28` server that answers
//! `tools/call` with an MRTR `input_required` result carrying a URL mode
//! `elicitation/create`, and asserts the client's half of the contract: declare
//! the capability only when a host can reach a human, get consent before
//! opening anything, echo `requestState` and `inputResponses` on the retry,
//! never hand a non-https URL to a consent surface, and stop asking after a
//! bounded number of rounds.

use async_trait::async_trait;
use everruns_core::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
};
use everruns_mcp::{
    ConsentingUrlElicitations, ElicitationAction, ElicitationConsentStore, GrantedConsent,
    McpClient, McpConnection, McpExecutor, NoAuthProvider, RelayUrlElicitations,
    StaticConnectionResolver, StoredConsent, UrlElicitation, UrlElicitationHandler,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

// Public IP literal: passes SSRF validation without DNS, never actually dialed.
const URL: &str = "http://8.8.8.8/mcp";
const ELICITATION_URL: &str = "https://mcp.example.com/connect?state=abc";

/// Server that elicits on every `tools/call` until `complete_after` calls have
/// been seen, then returns a real result.
struct ElicitingEgress {
    elicitation_url: &'static str,
    complete_after: usize,
    /// When set, the server completes as soon as a call answers `accept` —
    /// which is how a real one behaves: it keeps eliciting until the
    /// out-of-band interaction is done, then serves the call.
    honor_accept: bool,
    requests: Mutex<Vec<Value>>,
}

impl ElicitingEgress {
    fn new(elicitation_url: &'static str, complete_after: usize) -> Arc<Self> {
        Arc::new(Self {
            elicitation_url,
            complete_after,
            honor_accept: false,
            requests: Mutex::new(Vec::new()),
        })
    }

    /// A server that elicits forever until a call tells it the human consented.
    fn awaiting_accept(elicitation_url: &'static str) -> Arc<Self> {
        Arc::new(Self {
            elicitation_url,
            complete_after: usize::MAX,
            honor_accept: true,
            requests: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request["method"] == "tools/call")
            .cloned()
            .collect()
    }

    fn ok(body: Value) -> EgressResponse {
        EgressResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&body).unwrap(),
        }
    }
}

#[async_trait]
impl EgressService for ElicitingEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let parsed: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        self.requests.lock().unwrap().push(parsed.clone());
        if parsed["method"] != "tools/call" {
            return Ok(Self::ok(json!({ "jsonrpc": "2.0", "id": 1, "result": {} })));
        }
        let accepted = parsed["params"]["inputResponses"]
            .as_object()
            .is_some_and(|responses| {
                responses
                    .values()
                    .any(|response| response["action"] == "accept")
            });
        let prior = self.calls().len();
        if (self.honor_accept && accepted) || prior > self.complete_after {
            return Ok(Self::ok(json!({
                "jsonrpc": "2.0", "id": 1,
                "result": {
                    "resultType": "complete",
                    "content": [{ "type": "text", "text": "ok" }],
                    "isError": false
                }
            })));
        }
        Ok(Self::ok(json!({
            "jsonrpc": "2.0", "id": 1,
            "result": {
                "resultType": "input_required",
                "requestState": "opaque-state",
                "inputRequests": {
                    "connect": {
                        "method": "elicitation/create",
                        "params": {
                            "mode": "url",
                            "url": self.elicitation_url,
                            "message": "Authorize Example Co to continue."
                        }
                    }
                }
            }
        })))
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        unimplemented!("not used by MCP transport")
    }
}

/// Records what it was asked to consent to and answers with a fixed action.
struct ScriptedConsent {
    action: ElicitationAction,
    seen: Mutex<Vec<UrlElicitation>>,
}

impl ScriptedConsent {
    fn new(action: ElicitationAction) -> Arc<Self> {
        Arc::new(Self {
            action,
            seen: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl UrlElicitationHandler for ScriptedConsent {
    async fn request_url_consent(
        &self,
        elicitation: &UrlElicitation,
    ) -> anyhow::Result<ElicitationAction> {
        self.seen.lock().unwrap().push(elicitation.clone());
        Ok(self.action)
    }
}

fn client(egress: Arc<ElicitingEgress>, consent: Option<Arc<ScriptedConsent>>) -> McpClient {
    match consent {
        Some(consent) => McpClient::with_url_elicitation(egress, Arc::new(NoAuthProvider), consent),
        None => McpClient::new(egress, Arc::new(NoAuthProvider)),
    }
}

fn capabilities(call: &Value) -> &Value {
    &call["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"]
}

#[tokio::test]
async fn consent_retries_the_call_with_the_accept_response() {
    let egress = ElicitingEgress::new(ELICITATION_URL, 1);
    let consent = ScriptedConsent::new(ElicitationAction::Accept);
    let result = client(egress.clone(), Some(consent.clone()))
        .call(&McpConnection::http("docs", URL), "search", json!({}))
        .await
        .expect("accepted elicitation should complete the call");
    assert!(!result.is_error);

    // The human saw the full URL, its host, and which server asked.
    let seen = consent.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, ELICITATION_URL);
    assert_eq!(seen[0].host, "mcp.example.com");
    assert_eq!(seen[0].server_name, "docs");
    assert_eq!(seen[0].tool_name, "search");
    assert!(!seen[0].punycode);

    let calls = egress.calls();
    assert_eq!(calls.len(), 2, "exactly one retry");
    // Capability declared on every request, since this host can reach a human.
    assert_eq!(
        capabilities(&calls[0]),
        &json!({ "elicitation": { "url": {} } })
    );
    // The retry echoes the opaque state verbatim, answers under the server's
    // own key, and is an independent request (different JSON-RPC id).
    assert_eq!(calls[1]["params"]["requestState"], "opaque-state");
    assert_eq!(
        calls[1]["params"]["inputResponses"]["connect"],
        json!({ "action": "accept" })
    );
    assert_ne!(calls[0]["id"], calls[1]["id"]);
}

#[tokio::test]
async fn a_declined_elicitation_fails_the_call_without_retrying() {
    let egress = ElicitingEgress::new(ELICITATION_URL, 1);
    let consent = ScriptedConsent::new(ElicitationAction::Decline);
    let error = client(egress.clone(), Some(consent))
        .call(&McpConnection::http("docs", URL), "search", json!({}))
        .await
        .expect_err("a declined elicitation must fail the call");
    assert!(
        error.to_string().contains("mcp.example.com"),
        "unexpected: {error}"
    );
    // Typed, so a host can render the URL rather than parse a string.
    let pending = error
        .downcast_ref::<everruns_mcp::UrlElicitationPending>()
        .expect("typed pending elicitation");
    assert_eq!(pending.action, ElicitationAction::Decline);
    assert_eq!(pending.url, ELICITATION_URL);
    assert_eq!(
        egress.calls().len(),
        1,
        "nothing to send, so no retry should go out"
    );
}

#[tokio::test]
async fn a_host_with_no_handler_declares_nothing_and_refuses_the_elicitation() {
    let egress = ElicitingEgress::new(ELICITATION_URL, 1);
    let error = client(egress.clone(), None)
        .call(&McpConnection::http("docs", URL), "search", json!({}))
        .await
        .expect_err("a host with no human must not answer an elicitation");
    assert!(
        error.to_string().contains("no elicitation capability"),
        "unexpected: {error}"
    );
    let calls = egress.calls();
    assert_eq!(calls.len(), 1);
    // The declaration is what makes the server's request out of contract.
    assert_eq!(capabilities(&calls[0]), &json!({}));
}

#[tokio::test]
async fn a_non_https_elicitation_url_never_reaches_the_consent_surface() {
    let egress = ElicitingEgress::new("http://mcp.example.com/connect", 1);
    let consent = ScriptedConsent::new(ElicitationAction::Accept);
    let error = client(egress.clone(), Some(consent.clone()))
        .call(&McpConnection::http("docs", URL), "search", json!({}))
        .await
        .expect_err("non-https elicitation URLs must be rejected");
    assert!(
        error.to_string().contains("must use https"),
        "unexpected: {error}"
    );
    assert!(
        consent.seen.lock().unwrap().is_empty(),
        "the URL must be validated before a human is asked"
    );
}

#[tokio::test]
async fn a_server_that_keeps_eliciting_is_bounded() {
    // Never completes: every call comes back as another elicitation.
    let egress = ElicitingEgress::new(ELICITATION_URL, usize::MAX);
    let consent = ScriptedConsent::new(ElicitationAction::Accept);
    let error = client(egress.clone(), Some(consent))
        .call(&McpConnection::http("docs", URL), "search", json!({}))
        .await
        .expect_err("an endless elicitation loop must terminate");
    assert!(
        error.to_string().contains("still requires input"),
        "unexpected: {error}"
    );
    assert_eq!(
        egress.calls().len(),
        3,
        "initial call plus the bounded number of retries"
    );
}

#[tokio::test]
async fn the_relay_host_hands_the_user_the_url_as_an_actionable_result() {
    // What a real session host does: it cannot block a turn on a browser
    // interaction, so it declares the capability, never consents, and surfaces
    // the elicitation through the tool result for the user to act on.
    let egress = ElicitingEgress::new(ELICITATION_URL, usize::MAX);
    let client = Arc::new(McpClient::with_url_elicitation(
        egress.clone(),
        Arc::new(NoAuthProvider),
        Arc::new(RelayUrlElicitations),
    ));
    let executor = McpExecutor::new(
        client,
        Arc::new(StaticConnectionResolver::from_connections([
            McpConnection::http("docs", URL),
        ])),
    );

    let result = everruns_core::McpToolInvoker::invoke(
        &executor,
        &everruns_provider::tool_types::ToolCall {
            id: "call_1".to_string(),
            name: "mcp_docs__search".to_string(),
            arguments: json!({}),
        },
    )
    .await
    .expect("a pending elicitation is a result, not a failure");

    let payload = result.result.expect("structured result");
    assert_eq!(payload["code"], "url_elicitation_required");
    assert_eq!(payload["url"], ELICITATION_URL);
    assert_eq!(payload["url_host"], "mcp.example.com");
    assert_eq!(payload["declined"], false);
    assert!(payload["error"].as_str().unwrap().contains(ELICITATION_URL));
    // Not a transport failure: the model should relay it, not retry blindly.
    assert!(result.error.is_none());
}

/// The consent a user gave, as the host records it between the two calls.
#[derive(Default)]
struct RecordedConsents {
    records: Mutex<Vec<StoredConsent>>,
}

impl RecordedConsents {
    fn record(&self, consent: StoredConsent) {
        self.records.lock().unwrap().push(consent);
    }
}

#[async_trait]
impl ElicitationConsentStore for RecordedConsents {
    async fn take_consent(
        &self,
        server: &str,
        tool: &str,
    ) -> anyhow::Result<Option<GrantedConsent>> {
        let mut records = self.records.lock().unwrap();
        let found = records
            .iter()
            .position(|record| record.server == server && record.tool == tool);
        Ok(found.and_then(|index| {
            records
                .remove(index)
                .grant_for(server, tool, chrono::Utc::now())
        }))
    }
}

#[tokio::test]
async fn a_consenting_host_pauses_once_and_then_answers_accept() {
    // The full session shape: the first call has nothing to go on and stands
    // down with the URL, the user consents out of band, and the next run of the
    // same tool answers the server `accept`.
    let egress = ElicitingEgress::awaiting_accept(ELICITATION_URL);
    let consents = Arc::new(RecordedConsents::default());
    let client = Arc::new(McpClient::with_url_elicitation(
        egress.clone(),
        Arc::new(NoAuthProvider),
        Arc::new(ConsentingUrlElicitations::new(consents.clone())),
    ));
    let executor = McpExecutor::new(
        client,
        Arc::new(StaticConnectionResolver::from_connections([
            McpConnection::http("docs", URL),
        ])),
    );
    let tool_call = everruns_provider::tool_types::ToolCall {
        id: "call_1".to_string(),
        name: "mcp_docs__search".to_string(),
        arguments: json!({}),
    };

    let paused = everruns_core::McpToolInvoker::invoke(&executor, &tool_call)
        .await
        .expect("a pending elicitation is a result, not a failure");
    let payload = paused.result.expect("structured result");
    assert_eq!(payload["code"], "url_elicitation_required");
    assert_eq!(payload["server"], "docs");
    assert_eq!(payload["tool"], "search");
    // What the engine needs to name the call to re-run after consent.
    assert_eq!(payload["retry_tool"], "mcp_docs__search");
    assert_eq!(payload["url_host"], "mcp.example.com");

    // The user clicks through; the host records what they consented to.
    consents.record(StoredConsent::new(
        payload["server"].as_str().unwrap(),
        payload["tool"].as_str().unwrap(),
        payload["url_host"].as_str().unwrap(),
        chrono::Utc::now(),
    ));

    let completed = everruns_core::McpToolInvoker::invoke(&executor, &tool_call)
        .await
        .expect("the retry runs the tool");
    assert!(completed.error.is_none());
    assert!(
        everruns_provider::tool_types::UrlElicitationRequired::from_tool_result(&completed)
            .is_none(),
        "the second call must not stand down again"
    );

    let calls = egress.calls();
    assert_eq!(
        calls.len(),
        3,
        "one paused call, then the retry and its accept"
    );
    // Only the round that had consent behind it answered.
    assert!(calls[0]["params"]["inputResponses"].is_null());
    assert!(calls[1]["params"]["inputResponses"].is_null());
    assert_eq!(
        calls[2]["params"]["inputResponses"]["connect"],
        json!({ "action": "accept" })
    );
    assert_eq!(calls[2]["params"]["requestState"], "opaque-state");
}

#[tokio::test]
async fn a_consent_for_another_domain_is_not_reused() {
    let egress = ElicitingEgress::awaiting_accept(ELICITATION_URL);
    let consents = Arc::new(RecordedConsents::default());
    // Consent given for a different domain than the server now elicits.
    consents.record(StoredConsent::new(
        "docs",
        "search",
        "not-the-domain-you-saw.example",
        chrono::Utc::now(),
    ));
    let client = McpClient::with_url_elicitation(
        egress.clone(),
        Arc::new(NoAuthProvider),
        Arc::new(ConsentingUrlElicitations::new(consents)),
    );

    let error = client
        .call(&McpConnection::http("docs", URL), "search", json!({}))
        .await
        .expect_err("a consent for another domain must not answer this elicitation");
    assert!(
        error
            .downcast_ref::<everruns_mcp::UrlElicitationPending>()
            .is_some(),
        "unexpected: {error}"
    );
    assert!(
        egress.calls()[0]["params"]["inputResponses"].is_null(),
        "nothing may be answered without matching consent"
    );
}
