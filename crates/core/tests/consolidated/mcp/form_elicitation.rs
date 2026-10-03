#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Form mode elicitation integration tests
//! (knowledge/integrations/mcp-form-elicitation.md).
//!
//! A scripted [`EgressService`] plays a `2026-07-28` server that answers
//! `tools/call` with an MRTR `input_required` form `elicitation/create` until a
//! retry answers it, and asserts the client's half: declare `form` only where
//! the server's policy allows it, stand the call down with `ask_user` questions,
//! and send the recorded answer, typed, exactly once.

use async_trait::async_trait;
use everruns_core::mcp::{
    ConsentingUrlElicitations, ElicitationConsentStore, FormAnswer, FormAnswerAction,
    FormAnswerStore, GrantedConsent, McpClient, McpConnection, McpExecutor, NoAuthProvider,
    StaticConnectionResolver, StoredFormAnswer, StoredFormAnswers,
};
use everruns_core::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
    McpElicitationPolicy,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

// Public IP literal: passes SSRF validation without DNS, never actually dialed.
const URL: &str = "http://8.8.8.8/mcp";

fn requested_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "environment": { "type": "string", "enum": ["staging", "production"] },
            "replicas": { "type": "integer", "minimum": 1, "maximum": 5 }
        },
        "required": ["environment"]
    })
}

/// Elicits a form on every `tools/call` until one carries an answer.
#[derive(Default)]
struct FormEgress {
    requests: Mutex<Vec<Value>>,
}

impl FormEgress {
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
impl EgressService for FormEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let parsed: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        self.requests.lock().unwrap().push(parsed.clone());
        if parsed["method"] != "tools/call" {
            return Ok(Self::ok(json!({ "jsonrpc": "2.0", "id": 1, "result": {} })));
        }
        if let Some(answer) = parsed["params"]["inputResponses"].get("deploy") {
            return Ok(Self::ok(json!({
                "jsonrpc": "2.0", "id": 1,
                "result": {
                    "resultType": "complete",
                    "content": [{ "type": "text", "text": answer.to_string() }],
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
                    "deploy": {
                        "method": "elicitation/create",
                        "params": {
                            "mode": "form",
                            "message": "Where should this release go?",
                            "requestedSchema": requested_schema()
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

/// In-memory stand-in for the session store the server writes answers into.
#[derive(Default)]
struct Answers {
    records: Mutex<Vec<StoredFormAnswer>>,
}

#[async_trait]
impl FormAnswerStore for Answers {
    async fn take_form_answer(
        &self,
        server: &str,
        tool: &str,
    ) -> anyhow::Result<Option<StoredFormAnswer>> {
        let mut records = self.records.lock().unwrap();
        let found = records
            .iter()
            .position(|record| record.server == server && record.tool == tool);
        Ok(found.map(|index| records.remove(index)))
    }
}

#[async_trait]
impl ElicitationConsentStore for Answers {
    async fn take_consent(&self, _: &str, _: &str) -> anyhow::Result<Option<GrantedConsent>> {
        Ok(None)
    }
}

fn executor(
    egress: Arc<FormEgress>,
    answers: Arc<Answers>,
    policy: McpElicitationPolicy,
) -> McpExecutor {
    let client = Arc::new(McpClient::with_url_and_form_elicitation(
        egress,
        Arc::new(NoAuthProvider),
        Arc::new(ConsentingUrlElicitations::new(answers.clone())),
        Arc::new(StoredFormAnswers::new(answers)),
    ));
    McpExecutor::new(
        client,
        Arc::new(StaticConnectionResolver::from_connections([
            McpConnection::http("deploys", URL).with_elicitation_policy(policy),
        ])),
    )
}

fn tool_call() -> everruns_contracts::tool_types::ToolCall {
    everruns_contracts::tool_types::ToolCall {
        id: "call_1".to_string(),
        name: "mcp_deploys__release".to_string(),
        arguments: json!({}),
    }
}

fn capabilities(call: &Value) -> &Value {
    &call["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"]
}

#[tokio::test]
async fn a_form_pauses_once_and_the_recorded_answer_is_sent_typed() {
    let egress = Arc::new(FormEgress::default());
    let answers = Arc::new(Answers::default());
    let executor = executor(
        egress.clone(),
        answers.clone(),
        McpElicitationPolicy::UrlAndForm,
    );

    let paused = everruns_core::McpToolInvoker::invoke(&executor, &tool_call())
        .await
        .expect("a pending form is a result, not a failure");
    let pending =
        everruns_contracts::tool_types::FormElicitationRequired::from_tool_result(&paused)
            .expect("the call stands down with questions");
    assert_eq!(pending.server, "deploys");
    assert_eq!(pending.tool, "release");
    assert_eq!(pending.retry_tool, "mcp_deploys__release");
    assert_eq!(pending.message, "Where should this release go?");
    assert_eq!(pending.questions.len(), 2);
    // Everruns, not the server, names who is asking.
    assert!(
        pending
            .questions
            .iter()
            .all(|question| question["header"] == "deploys")
    );

    // The person answers; the server's answer API records it.
    answers.records.lock().unwrap().push(StoredFormAnswer::new(
        &pending.server,
        &pending.tool,
        &pending.fingerprint,
        FormAnswerAction::Accept,
        BTreeMap::from([
            (
                "environment".to_string(),
                FormAnswer {
                    selected: vec!["production".to_string()],
                    other_text: None,
                },
            ),
            (
                "replicas".to_string(),
                FormAnswer {
                    selected: vec![],
                    other_text: Some("3".to_string()),
                },
            ),
        ]),
        chrono::Utc::now(),
    ));

    let completed = everruns_core::McpToolInvoker::invoke(&executor, &tool_call())
        .await
        .expect("the retry runs the tool");
    assert!(completed.error.is_none());

    let calls = egress.calls();
    assert_eq!(
        calls.len(),
        3,
        "one paused call, then the retry and its answer"
    );
    assert_eq!(
        capabilities(&calls[0]),
        &json!({ "elicitation": { "url": {}, "form": {} } })
    );
    assert!(calls[1]["params"]["inputResponses"].is_null());
    assert_eq!(
        calls[2]["params"]["inputResponses"]["deploy"],
        json!({
            "action": "accept",
            "content": { "environment": "production", "replicas": 3 }
        })
    );
    assert_eq!(calls[2]["params"]["requestState"], "opaque-state");
    assert!(
        answers.records.lock().unwrap().is_empty(),
        "an answer is consumed by the call that sends it"
    );
}

#[tokio::test]
async fn a_recorded_decline_is_sent_as_a_decline() {
    let egress = Arc::new(FormEgress::default());
    let answers = Arc::new(Answers::default());
    let executor = executor(
        egress.clone(),
        answers.clone(),
        McpElicitationPolicy::UrlAndForm,
    );
    let paused = everruns_core::McpToolInvoker::invoke(&executor, &tool_call())
        .await
        .unwrap();
    let pending =
        everruns_contracts::tool_types::FormElicitationRequired::from_tool_result(&paused).unwrap();
    answers.records.lock().unwrap().push(StoredFormAnswer::new(
        &pending.server,
        &pending.tool,
        &pending.fingerprint,
        FormAnswerAction::Decline,
        BTreeMap::new(),
        chrono::Utc::now(),
    ));

    everruns_core::McpToolInvoker::invoke(&executor, &tool_call())
        .await
        .unwrap();

    let calls = egress.calls();
    assert_eq!(
        calls.last().unwrap()["params"]["inputResponses"]["deploy"],
        json!({ "action": "decline" })
    );
}

#[tokio::test]
async fn an_answer_to_other_questions_is_not_sent() {
    let egress = Arc::new(FormEgress::default());
    let answers = Arc::new(Answers::default());
    let executor = executor(
        egress.clone(),
        answers.clone(),
        McpElicitationPolicy::UrlAndForm,
    );
    answers.records.lock().unwrap().push(StoredFormAnswer::new(
        "deploys",
        "release",
        "a-different-schema",
        FormAnswerAction::Accept,
        BTreeMap::new(),
        chrono::Utc::now(),
    ));

    let result = everruns_core::McpToolInvoker::invoke(&executor, &tool_call())
        .await
        .unwrap();

    assert!(
        everruns_contracts::tool_types::FormElicitationRequired::from_tool_result(&result)
            .is_some(),
        "a stale answer asks again instead of answering"
    );
    assert!(
        egress
            .calls()
            .iter()
            .all(|call| call["params"]["inputResponses"].is_null())
    );
}

#[tokio::test]
async fn a_server_without_form_policy_is_never_told_forms_work() {
    let egress = Arc::new(FormEgress::default());
    let executor = executor(
        egress.clone(),
        Arc::new(Answers::default()),
        McpElicitationPolicy::Url,
    );

    let error = everruns_core::McpToolInvoker::invoke(&executor, &tool_call())
        .await
        .expect_err("a form the policy forbids never reaches a person");
    assert!(error.to_string().contains("did not declare form mode"));
    let calls = egress.calls();
    assert_eq!(
        capabilities(&calls[0]),
        &json!({ "elicitation": { "url": {} } })
    );
    assert!(
        calls
            .iter()
            .all(|call| call["params"]["inputResponses"].is_null())
    );
}
