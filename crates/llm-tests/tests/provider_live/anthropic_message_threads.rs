//! Real Messages API acceptance, including delta-only turns and recovery from
//! a consumed response ID. No mock responses, skipped credentials, or unthreaded
//! success: the captured request must prove the thread path actually ran.
//!
//! Run only these paid, synthetic cases with:
//! `doppler run -- cargo test -p everruns-llm-tests --test provider_live anthropic_message_threads -- --nocapture`

use async_trait::async_trait;
use everruns_contracts::driver_registry::{LlmContentPart, LlmResponse};
use everruns_contracts::tool_types::ToolDefinition;
use everruns_contracts::{
    LlmCallConfig, Message, MessageRole, Provider, ProviderAuth, ProviderAuthRequest,
};
use everruns_drivers::anthropic::AnthropicChatDriver;
use rstest::rstest;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const URL: &str = "https://api.anthropic.com/v1/messages";
const BETA: &str = "message-threads-2026-08-12";
const MODEL: &str = "claude-haiku-4-5-20251001";

// The auth boundary sees the actual transformed bytes before HTTP dispatch.
// Record only synthetic bodies, never credentials or headers. This lets the
// recovery test distinguish provider rejection/replay from a local cache miss.
struct RecordingAuth {
    key: String,
    requests: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl ProviderAuth for RecordingAuth {
    async fn headers(
        &self,
        request: ProviderAuthRequest<'_>,
    ) -> everruns_contracts::Result<Vec<(String, String)>> {
        assert_eq!(request.url, URL);
        let body: Value = serde_json::from_slice(request.body).unwrap();
        if body.get("thread").is_some() {
            self.requests.lock().unwrap().push(body);
        }
        Ok(vec![("x-api-key".into(), self.key.clone())])
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn api_key() -> String {
    std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY is required for live threads")
}

fn live_provider() -> (Provider, Arc<Mutex<Vec<Value>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Provider::new("anthropic", AnthropicChatDriver::new())
        .base_url("https://api.anthropic.com/v1")
        .auth(RecordingAuth {
            key: api_key(),
            requests: requests.clone(),
        });
    (provider, requests)
}

fn config(native_json: bool) -> LlmCallConfig {
    let mut config = LlmCallConfig::new(MODEL);
    config.max_tokens = Some(256);
    config.temperature = Some(0.0);
    config.prefer_non_streaming = native_json;
    config.capture_request = true;
    config
        .metadata
        .insert("session_id".into(), uuid::Uuid::new_v4().to_string());
    config
        .driver_options
        .insert("anthropic/message_threads".into(), json!(true));
    config
}

async fn call(provider: &Provider, messages: &[Message], config: &LlmCallConfig) -> LlmResponse {
    let response = tokio::time::timeout(
        Duration::from_secs(90),
        provider.chat_completion(messages.to_vec(), config),
    )
    .await
    .expect("live thread turn exceeded 90 seconds")
    .expect("real Anthropic thread request must succeed");
    assert!(
        response
            .metadata
            .response_id
            .as_deref()
            .unwrap()
            .starts_with("msg_")
    );
    assert!(matches!(
        response.metadata.provider_finish_reason.as_deref(),
        Some("end_turn" | "tool_use")
    ));
    response
}

fn remember(token: &str) -> Vec<Message> {
    vec![
        Message::text(
            MessageRole::System,
            "Follow the user's instructions precisely. Keep replies short.",
        ),
        Message::text(
            MessageRole::User,
            format!("Remember this synthetic token: {token}. Reply only ACK."),
        ),
    ]
}

fn append_assistant(messages: &mut Vec<Message>, response: &LlmResponse) {
    let mut message = Message::parts(
        MessageRole::Assistant,
        vec![LlmContentPart::ProviderOpaque(
            response.metadata.provider_opaque_content.clone().unwrap(),
        )],
    );
    message.tool_calls = response.tool_calls.clone();
    messages.push(message);
}

fn assert_thread(response: &LlmResponse, previous: Option<&LlmResponse>, native_json: bool) {
    let body = response
        .metadata
        .request_body
        .as_ref()
        .expect("capture actual request");
    assert_eq!(body["stream"], !native_json);
    assert_eq!(
        body["thread"]["type"],
        if previous.is_some() {
            "continue"
        } else {
            "create"
        },
        "a successful replay/unthreaded fallback does not prove continuation"
    );
    if let Some(previous) = previous {
        assert_eq!(
            body["thread"]["previous_message_id"],
            previous.metadata.response_id.as_deref().unwrap()
        );
        assert_ne!(response.metadata.response_id, previous.metadata.response_id);
        assert!(body.get("system").is_none());
        assert!(body.get("tools").is_none());
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    } else {
        assert!(body.get("system").is_some());
    }
}

#[rstest]
#[case::streaming(false)]
#[case::native_json(true)]
#[tokio::test]
async fn real_threads_retain_context_across_delta_only_turns(#[case] native_json: bool) {
    let (provider, _) = live_provider();
    let config = config(native_json);
    let token = uuid::Uuid::new_v4().to_string();
    let mut messages = remember(&token);
    let mut previous = call(&provider, &messages, &config).await;
    assert_thread(&previous, None, native_json);
    for _ in 0..2 {
        append_assistant(&mut messages, &previous);
        messages.push(Message::text(
            MessageRole::User,
            "Return only the token I asked you to remember.",
        ));
        let response = call(&provider, &messages, &config).await;
        assert_thread(&response, Some(&previous), native_json);
        assert!(
            !response.metadata.request_body.as_ref().unwrap()["messages"]
                .to_string()
                .contains(&token),
            "the token must come from provider state, not the outgoing delta"
        );
        assert!(
            response.text.contains(&token),
            "provider must recover the token from its thread"
        );
        previous = response;
    }
    println!(
        "live threads: native_json={native_json}, create + two delta continuations retained context"
    );
}

#[tokio::test]
async fn real_thread_continues_with_tool_result_without_resending_tools() {
    let (provider, _) = live_provider();
    let mut config = config(false);
    config.tools = vec![ToolDefinition::function(
        "lookup_code",
        "Return the synthetic verification code.",
        json!({"type":"object","properties":{},"additionalProperties":false}),
    )];
    let mut messages = vec![
        Message::text(
            MessageRole::System,
            "Call lookup_code exactly once to answer. After its result, return the code verbatim and do not call tools again.",
        ),
        Message::text(
            MessageRole::User,
            "What is the verification code? Use lookup_code.",
        ),
    ];
    let first = call(&provider, &messages, &config).await;
    assert_thread(&first, None, false);
    assert!(first.metadata.request_body.as_ref().unwrap()["tools"].is_array());
    let calls = first
        .tool_calls
        .as_ref()
        .expect("real model must call the tool");
    assert_eq!(
        first.metadata.provider_finish_reason.as_deref(),
        Some("tool_use")
    );
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "lookup_code");
    let token = uuid::Uuid::new_v4().to_string();
    append_assistant(&mut messages, &first);
    let mut result = Message::text(MessageRole::Tool, &token);
    result.tool_call_id = Some(calls[0].id.clone());
    messages.push(result);
    let second = call(&provider, &messages, &config).await;
    assert_thread(&second, Some(&first), false);
    let body = second.metadata.request_body.as_ref().unwrap();
    assert_eq!(body["messages"][0]["content"][0]["type"], "tool_result");
    assert_eq!(
        body["messages"][0]["content"][0]["tool_use_id"],
        calls[0].id
    );
    assert!(second.text.contains(&token));
    assert_eq!(
        second.metadata.provider_finish_reason.as_deref(),
        Some("end_turn")
    );
    assert!(second.tool_calls.as_ref().is_none_or(Vec::is_empty));
    println!("live threads: tool-result delta accepted with tools and assistant call omitted");
}

async fn raw_request(body: &Value) -> (reqwest::StatusCode, Value) {
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .post(URL)
        .header("x-api-key", api_key())
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", BETA)
        .json(body)
        .send()
        .await
        .expect("real Messages request");
    let status = response.status();
    (
        status,
        response.json().await.expect("real Messages JSON response"),
    )
}

#[tokio::test]
async fn real_consumed_thread_replays_history_then_continues_again() {
    let (provider, requests) = live_provider();
    let config = config(true);
    let token = uuid::Uuid::new_v4().to_string();
    let mut messages = remember(&token);
    let first = call(&provider, &messages, &config).await;
    assert_thread(&first, None, true);

    // Consume the ID outside this driver, reproducing a response lost by a
    // client after the server committed it. Both the success and the subsequent
    // rejection below come from Anthropic, not an injected error fixture.
    let mut continuation = first.metadata.request_body.clone().unwrap();
    continuation.as_object_mut().unwrap().remove("system");
    continuation.as_object_mut().unwrap().remove("tools");
    continuation["thread"] =
        json!({"type":"continue","previous_message_id":first.metadata.response_id});
    continuation["messages"] =
        json!([{"role":"user","content":"Return only the token I asked you to remember."}]);
    let (status, response) = raw_request(&continuation).await;
    assert!(
        status.is_success(),
        "real continuation rejected: {status} {response}"
    );
    assert_eq!(response["stop_reason"], "end_turn");
    assert!(response["content"].to_string().contains(&token));
    let (status, error) = raw_request(&continuation).await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(
        error["error"]["details"]["error_code"], "thread_already_continued",
        "{error}"
    );

    append_assistant(&mut messages, &first);
    messages.push(Message::text(
        MessageRole::User,
        "Return only the token I asked you to remember.",
    ));
    let replay = call(&provider, &messages, &config).await;
    assert_thread(&replay, None, true);
    assert!(replay.text.contains(&token));
    let attempts = requests.lock().unwrap().clone();
    assert_eq!(
        attempts.len(),
        3,
        "create, rejected continue, exactly one replay"
    );
    assert_eq!(attempts[1]["thread"], continuation["thread"]);
    assert_eq!(attempts[1]["messages"].as_array().unwrap().len(), 1);
    assert_eq!(attempts[2]["messages"].as_array().unwrap().len(), 3);
    assert!(attempts[2]["messages"].to_string().contains(&token));

    append_assistant(&mut messages, &replay);
    messages.push(Message::text(MessageRole::User, "Repeat the token only."));
    let last = call(&provider, &messages, &config).await;
    assert_thread(&last, Some(&replay), true);
    assert!(last.text.contains(&token));
    println!("live threads: real thread_already_continued -> full replay -> delta continuation");
}
