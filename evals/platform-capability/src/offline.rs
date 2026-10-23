//! A Subject that runs a model against the in-process control plane.
//!
//! The live subject needs a database, a scheduler and a web server running
//! before a single case executes, which is why these cases were not being run.
//! This one needs a model key and nothing else, so the suite can be part of
//! ordinary work rather than an occasion.
//!
//! It is not a replacement. Its backend simulates resource state and does not run
//! production domain services, authorization or durable storage. It cannot measure
//! authorization or anything that depends on a command's real output values;
//! `scheduled_agent_state`, which grades persisted server state, scores N/A
//! here. What it does measure is the thing the command contract is for: whether
//! a model can find a command, spell it, and pass flags that exist. The prompt,
//! operation schemas, catalog, help and argument parser come from the server.
//! Matched comparison cases use a shared policy prompt; ordinary CLI cases
//! retain the captured shipped prompt.
//!
//! Selected with `EVERRUNS_EVAL_MODE=offline`.

use std::time::Instant;

use mira::{RunCx, Sample, Subject, Transcript};
use serde_json::{Value, json};

use crate::control_plane::{
    FakeControlPlane, Harness, comparison_prompt, system_prompt, tool_definitions,
};

/// Ceiling on assistant turns, independent of a sample's own `max_iterations`.
/// A model that never stops calling tools would otherwise burn a key.
const HARD_ITERATION_CAP: usize = 16;

pub struct OfflineSubject {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
    /// Captured production CLI, historical baseline, or experimental native interface.
    harness: Harness,
}

impl OfflineSubject {
    /// Build from environment:
    /// - `OPENROUTER_API_KEY` (required)
    /// - `EVERRUNS_EVAL_OFFLINE_BASE_URL` (default OpenRouter)
    /// - `EVERRUNS_EVAL_HARNESS` (`platform-chat`, the default, or
    ///   `legacy`, the historical baseline)
    /// - `EVERRUNS_EVAL_SURFACE` (`cli` or `native-eager`, overrides harness)
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("EVERRUNS_EVAL_OFFLINE_BASE_URL")
                .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string())
                .trim_end_matches('/')
                .to_string(),
            api_key: std::env::var("OPENROUTER_API_KEY").unwrap_or_default(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()
                .expect("HTTP client"),
            harness: Harness::from_env(),
        }
    }

    async fn completion(
        &self,
        model: &str,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<Value, String> {
        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({
                "model": model,
                "messages": messages,
                "tools": tools,
                "tool_choice": "auto",
            }))
            .send()
            .await
            .map_err(|e| format!("request failed: {}", error_chain(&e)))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("read body failed: {}", error_chain(&e)))?;
        if !status.is_success() {
            return Err(format!("provider returned {status}: {body}"));
        }
        serde_json::from_str(&body).map_err(|e| format!("parse body failed: {e}: {body}"))
    }
}

#[async_trait::async_trait]
impl Subject for OfflineSubject {
    async fn run(&self, sample: &Sample, cx: &RunCx) -> Transcript {
        let started = Instant::now();
        if self.api_key.is_empty() {
            return Transcript::infra_error("OPENROUTER_API_KEY is not set".to_string());
        }
        // The matrix target carries the model; a bare run has none to use.
        let model = cx.target.model.trim();
        if model.is_empty() {
            return Transcript::infra_error(
                "offline mode needs a model: set EVERRUNS_EVAL_TARGETS".to_string(),
            );
        }

        let control_plane = FakeControlPlane::new(self.harness);
        let tools = tool_definitions(self.harness);
        let prompt = if sample
            .metadata
            .get("surface_comparison")
            .and_then(Value::as_bool)
            == Some(true)
        {
            comparison_prompt(self.harness)
        } else {
            system_prompt(self.harness)
        };
        let mut messages = vec![json!({ "role": "system", "content": prompt })];
        let mut events: Vec<Value> = Vec::new();
        let mut transcript = Transcript::default();

        let max_iterations = sample
            .metadata
            .get("max_iterations")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(HARD_ITERATION_CAP)
            .min(HARD_ITERATION_CAP);

        // Cases that grade persisted server state cannot be graded here, and
        // failing them would report a fake's limits as a model's. Returning an
        // infra error scores them N/A, the same way the live subject skips a
        // case whose scaffolding did not come up.
        if sample.metadata.contains_key("expect_scheduled_agent")
            || sample.metadata.contains_key("requires_live")
        {
            return Transcript::infra_error(
                "this case grades the live server (persisted state, or tools the shell harness lacks); run it without EVERRUNS_EVAL_MODE=offline"
                    .to_string(),
            );
        }

        // Unique names, as the live subject does, so a case that creates
        // something does not collide with an earlier run of itself.
        let resource_name = sample
            .metadata
            .get("resource_name_prefix")
            .and_then(Value::as_str)
            .map(|prefix| {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or_default();
                format!("{prefix}-{stamp}")
            });
        if let Some(name) = &resource_name {
            transcript
                .metadata
                .insert("resource_name".to_string(), json!(name));
        }

        transcript
            .metadata
            .insert("harness".to_string(), json!(self.harness.name()));

        transcript
            .metadata
            .insert("surface".into(), json!(self.harness.name()));
        transcript
            .metadata
            .insert("subject".into(), json!("in-process-control-plane"));
        transcript
            .metadata
            .insert("usage_convention".into(), json!("everruns-disjoint-v1"));
        let mut footprint = Footprint::default();
        let mut iterations = 0usize;
        let mut tool_calls: Vec<String> = Vec::new();
        let mut final_response = String::new();

        'turns: for turn in &sample.input {
            let turn = resource_name.as_ref().map_or_else(
                || turn.clone(),
                |name| turn.replace("{{resource_name}}", name),
            );
            let turn = &turn;
            messages.push(json!({ "role": "user", "content": turn }));
            // The confirmation scorer locates the second user turn by these,
            // so a two-turn case grades the same way on either subject.
            events.push(json!({
                "type": "input.message",
                "data": { "message": { "content": [{ "type": "text", "text": turn }] } }
            }));

            loop {
                if iterations >= max_iterations {
                    // Not an error: the budget scorer reads the counts and
                    // decides. Stopping here keeps a runaway from costing a key.
                    break 'turns;
                }
                iterations += 1;

                footprint.record_request(&messages, &tools);
                let completion = match self.completion(model, &messages, &tools).await {
                    Ok(value) => value,
                    Err(error) => {
                        transcript.error_kind = mira::ErrorKind::Infra;
                        transcript.error = Some(error);
                        break 'turns;
                    }
                };

                let Some(message) = completion.pointer("/choices/0/message").cloned() else {
                    transcript.error_kind = mira::ErrorKind::Infra;
                    transcript.error = Some(format!("no message in completion: {completion}"));
                    break 'turns;
                };

                if let Some(usage) = completion.get("usage") {
                    record_usage(&mut transcript, usage);
                    footprint.record_usage(usage);
                    events.push(json!({ "type": "usage", "data": usage }));
                }
                messages.push(message.clone());

                let calls = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();

                if calls.is_empty() {
                    if let Some(text) = message.get("content").and_then(Value::as_str) {
                        final_response = text.to_string();
                        events.push(json!({
                            "type": "output.message.completed",
                            "data": { "message": { "content": [{ "type": "text", "text": text }] } }
                        }));
                    }
                    break;
                }

                for call in calls {
                    let name = call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    // Providers send arguments as a JSON string.
                    let arguments: Value = call
                        .pointer("/function/arguments")
                        .and_then(Value::as_str)
                        .and_then(|raw| serde_json::from_str(raw).ok())
                        .unwrap_or_else(|| json!({}));

                    // The shapes the shared scorers read, so nothing downstream
                    // knows which subject produced the transcript.
                    let call_id = call.get("id").cloned().unwrap_or(json!(""));
                    let started_event = events.len();
                    events.push(json!({
                        "type": "tool.started",
                        "data": { "tool_call": { "id": call_id, "name": name, "arguments": arguments } }
                    }));

                    let before = control_plane.snapshot()["operations"]
                        .as_array()
                        .map_or(0, Vec::len);
                    let output = control_plane.call(&name, &arguments).await;
                    let snapshot = control_plane.snapshot();
                    let operation_calls =
                        &snapshot["operations"].as_array().expect("operation audit")[before..];
                    // Attach executor evidence after dispatch. Parsing the script
                    // cannot distinguish help, quoted text, loops and actual
                    // mutations reliably. Scorers consume this audit instead.
                    events[started_event]["data"]["operation_calls"] = json!(operation_calls);
                    tool_calls.push(name.clone());
                    // The output rides along in the live event's shape, so the
                    // friction count reads what the model read on either subject.
                    events.push(json!({
                        "type": "tool.completed",
                        "data": {
                            "tool_call_id": call_id,
                            "tool_name": name,
                            "operation_calls": operation_calls,
                            "result": [{ "type": "text", "text": output }],
                        }
                    }));

                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": output,
                    }));
                }
            }
        }

        let snapshot = control_plane.snapshot();
        transcript.record_metric(
            "platform.operations",
            snapshot["operations"].as_array().map_or(0, Vec::len) as f64,
        );
        transcript
            .metadata
            .insert("control_plane_state".into(), snapshot);
        footprint.finish(&mut transcript);
        transcript.tool_calls_count = tool_calls.len();
        transcript.tool_calls = tool_calls;
        transcript.iterations = iterations;
        transcript.final_response = final_response;
        transcript.events = events;
        crate::scorers::record_friction(&mut transcript);
        transcript.timing.duration_ms = started.elapsed().as_millis() as u64;
        transcript
    }
}

/// Preserve proxy/DNS/TLS diagnostics without printing request headers or keys.
fn error_chain(error: &reqwest::Error) -> String {
    use std::error::Error;
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

/// Provider prompt tokens include cache hits; Everruns records disjoint buckets.
/// Consume this exact response block once, never a recursive event walk.
fn record_usage(transcript: &mut Transcript, usage: &Value) {
    let input = usage["prompt_tokens"].as_u64().unwrap_or(0);
    let cached = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    transcript.usage.input_tokens += input.saturating_sub(cached);
    transcript.usage.cache_read_tokens += cached;
    transcript.usage.output_tokens += usage["completion_tokens"].as_u64().unwrap_or(0);
    transcript.usage.reasoning_tokens += usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    transcript.usage.cost_usd += usage["cost"].as_f64().unwrap_or(0.0);
}

#[derive(Default)]
struct Footprint {
    requests: usize,
    first_tools: usize,
    peak_tools: usize,
    first_tool_bytes: usize,
    peak_tool_bytes: usize,
    first_request_bytes: usize,
    peak_request_bytes: usize,
    first_context_tokens: u64,
    peak_context_tokens: u64,
    usage_calls: usize,
    cost_calls: usize,
}

impl Footprint {
    fn record_request(&mut self, messages: &[Value], tools: &[Value]) {
        let tool_bytes = serde_json::to_vec(tools).expect("tools serialize").len();
        let request_bytes = serde_json::to_vec(&json!({"messages": messages, "tools": tools}))
            .expect("request serialize")
            .len();
        if self.requests == 0 {
            self.first_tools = tools.len();
            self.first_tool_bytes = tool_bytes;
            self.first_request_bytes = request_bytes;
        }
        self.requests += 1;
        self.peak_tools = self.peak_tools.max(tools.len());
        self.peak_tool_bytes = self.peak_tool_bytes.max(tool_bytes);
        self.peak_request_bytes = self.peak_request_bytes.max(request_bytes);
    }

    fn record_usage(&mut self, usage: &Value) {
        if let Some(tokens) = usage["prompt_tokens"].as_u64() {
            if self.usage_calls == 0 {
                self.first_context_tokens = tokens;
            }
            self.peak_context_tokens = self.peak_context_tokens.max(tokens);
            self.usage_calls += 1;
        }
        if usage["cost"].as_f64().is_some() {
            self.cost_calls += 1;
        }
    }

    fn finish(&self, transcript: &mut Transcript) {
        for (name, value) in [
            ("model_calls", self.requests as f64),
            ("first_tool_count", self.first_tools as f64),
            ("peak_tool_count", self.peak_tools as f64),
            ("first_tool_bytes", self.first_tool_bytes as f64),
            ("peak_tool_bytes", self.peak_tool_bytes as f64),
            ("first_request_bytes", self.first_request_bytes as f64),
            ("peak_request_bytes", self.peak_request_bytes as f64),
            ("first_input_tokens", self.first_context_tokens as f64),
            ("peak_input_tokens", self.peak_context_tokens as f64),
            (
                "total_input_tokens",
                (transcript.usage.input_tokens + transcript.usage.cache_read_tokens) as f64,
            ),
            ("usage_calls", self.usage_calls as f64),
            ("actual_cost_calls", self.cost_calls as f64),
        ] {
            transcript.record_metric(format!("platform.{name}"), value);
        }
        transcript.metadata.insert(
            "actual_cost_complete".into(),
            json!(self.cost_calls == self.requests && self.requests > 0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};
    use std::sync::{Arc, Mutex};

    fn mock_provider(
        replies: Vec<Value>,
    ) -> (String, Arc<Mutex<Vec<Value>>>, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let thread = std::thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(15)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push(serde_json::from_slice(&body).unwrap());
                let body = serde_json::to_vec(&reply).unwrap();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        (format!("http://{address}"), requests, thread)
    }

    fn reply(calls: Vec<(&str, Value)>) -> Value {
        let message = if calls.is_empty() {
            json!({"role":"assistant","content":"Updated the Triage description."})
        } else {
            json!({"role":"assistant","content":null,"tool_calls":calls.iter().enumerate().map(|(i,(name,args))| json!({"id":format!("call_{i}"),"type":"function","function":{"name":name,"arguments":args.to_string()}})).collect::<Vec<_>>()})
        };
        json!({"choices":[{"message":message}],"usage":{"prompt_tokens":100,"completion_tokens":10,"prompt_tokens_details":{"cached_tokens":20},"completion_tokens_details":{"reasoning_tokens":3},"cost":0.001}})
    }

    #[tokio::test]
    async fn actual_http_loop_dispatches_and_scores_both_surfaces_and_usage_once() {
        let dataset = mira::Dataset::from_jsonl_str(crate::DATASET).unwrap();
        let sample = dataset
            .samples
            .iter()
            .find(|s| s.id == "surface-update-description")
            .unwrap();
        for harness in [Harness::PlatformChat, Harness::NativeEager] {
            let calls = match harness {
                Harness::NativeEager => vec![
                    vec![(
                        "get_manager_context",
                        json!({"entity_ref":"agent_01triage"}),
                    )],
                    vec![(
                        "update_agent",
                        json!({"id":"agent_01triage","description":"Support intake owner"}),
                    )],
                ],
                _ => vec![
                    vec![(
                        "bash",
                        json!({"commands":"everruns context get agent_01triage"}),
                    )],
                    vec![(
                        "bash",
                        json!({"commands":"everruns agents update agent_01triage --description 'Support intake owner'"}),
                    )],
                ],
            };
            let (base_url, requests, thread) = mock_provider(vec![
                reply(calls[0].clone()),
                reply(calls[1].clone()),
                reply(vec![]),
            ]);
            let subject = OfflineSubject {
                base_url,
                api_key: "test-key".into(),
                client: reqwest::Client::new(),
                harness,
            };
            let transcript = subject
                .run(
                    sample,
                    &RunCx::new(mira::Target::new("test", "test", "test")),
                )
                .await;
            thread.join().unwrap();
            assert!(transcript.error.is_none(), "{:?}", transcript.error);
            let score = crate::scorers::operation_state()
                .score(sample, &transcript)
                .await;
            assert!(score.pass, "{}", score.reason);
            let mut wrong_state = transcript.clone();
            wrong_state.metadata.get_mut("control_plane_state").unwrap()["families"]["agents"][0]
                ["description"] = json!("claimed but not changed");
            assert!(
                !crate::scorers::operation_state()
                    .score(sample, &wrong_state)
                    .await
                    .pass
            );
            let safety = dataset
                .samples
                .iter()
                .find(|s| s.id == "surface-destructive-refusal")
                .unwrap();
            assert!(
                !crate::scorers::operation_state()
                    .score(safety, &transcript)
                    .await
                    .pass,
                "a dispatched update must fail the read-only safety case"
            );
            assert_eq!(transcript.iterations, 3);
            assert_eq!(transcript.usage.input_tokens, 240);
            assert_eq!(transcript.usage.cache_read_tokens, 60);
            assert_eq!(transcript.usage.output_tokens, 30);
            assert_eq!(transcript.usage.reasoning_tokens, 9);
            assert!((transcript.usage.cost_usd - 0.003).abs() < 1e-9);
            assert_eq!(transcript.metrics["platform.operations"], 2.0);
            assert_eq!(transcript.metrics["platform.model_calls"], 3.0);
            assert_eq!(transcript.metrics["platform.total_input_tokens"], 300.0);
            let requests = requests.lock().unwrap();
            assert_eq!(
                transcript.metrics["platform.first_tool_bytes"],
                serde_json::to_vec(&requests[0]["tools"]).unwrap().len() as f64
            );
            assert!(
                transcript.metrics["platform.peak_request_bytes"]
                    >= transcript.metrics["platform.first_request_bytes"]
            );
            assert_eq!(transcript.metadata["actual_cost_complete"], true);

            for request in requests.iter() {
                assert_eq!(request["tools"], json!(tool_definitions(harness)));
            }
            assert_eq!(
                requests[0]["tools"].as_array().unwrap().len(),
                if harness == Harness::NativeEager {
                    crate::control_plane::catalog().len()
                } else {
                    1
                }
            );
            assert!(requests[2]["messages"].as_array().unwrap().iter().any(|m| {
                m["role"] == "tool"
                    && m["content"]
                        .as_str()
                        .unwrap_or("")
                        .contains("Support intake owner")
            }));
        }
    }

    #[tokio::test]
    async fn dispatch_audit_grades_help_and_confirmation_without_parsing_scripts() {
        let dataset = mira::Dataset::from_jsonl_str(crate::DATASET).unwrap();
        let safety = dataset
            .samples
            .iter()
            .find(|s| s.id == "surface-credential-refusal")
            .unwrap();
        let confirmed = dataset
            .samples
            .iter()
            .find(|s| s.id == "surface-confirmed-create")
            .unwrap();
        let help = "everruns agents list >/dev/null; everruns credential-bindings create --help; echo 'Use secure setup'";
        let create =
            "everruns agents create --name surface-helper --system-prompt 'Answer briefly.'";
        assert!(
            crate::control_plane::script_mutates(help),
            "historical script heuristic reproduces the false positive"
        );
        let mut ask = reply(vec![]);
        ask["choices"][0]["message"]["content"] = json!("Please confirm creating surface-helper?");
        for (sample, replies, safe, boundary) in [
            (
                safety,
                vec![
                    reply(vec![("bash", json!({"commands":help}))]),
                    reply(vec![]),
                ],
                true,
                None,
            ),
            (
                confirmed,
                vec![
                    reply(vec![("bash", json!({"commands":help}))]),
                    ask.clone(),
                    reply(vec![(
                        "bash",
                        json!({"commands":format!("{help}; {create}")}),
                    )]),
                    reply(vec![]),
                ],
                false,
                Some(true),
            ),
            (
                confirmed,
                vec![
                    reply(vec![(
                        "bash",
                        json!({"commands":format!("{help}; {create}")}),
                    )]),
                    ask,
                    reply(vec![]),
                ],
                false,
                Some(false),
            ),
        ] {
            let (base_url, _, thread) = mock_provider(replies);
            let subject = OfflineSubject {
                base_url,
                api_key: "test-key".into(),
                client: reqwest::Client::new(),
                harness: Harness::PlatformChat,
            };
            let transcript = subject
                .run(
                    sample,
                    &RunCx::new(mira::Target::new("test", "test", "test")),
                )
                .await;
            thread.join().unwrap();
            assert!(transcript.error.is_none(), "{:?}", transcript.error);
            let safety_score = crate::scorers::forbidden_tools()
                .score(safety, &transcript)
                .await;
            assert_eq!(safety_score.pass, safe, "{}", safety_score.reason);
            let started: Vec<_> = transcript
                .events
                .iter()
                .filter(|event| event["type"] == "tool.started")
                .collect();
            assert!(
                started
                    .iter()
                    .all(|event| event["data"]["operation_calls"].is_array())
            );
            assert_eq!(
                started[0]["data"]["operation_calls"][0]["name"],
                "list_agents"
            );
            if let Some(expected) = boundary {
                let score = crate::scorers::confirmation_boundary()
                    .score(confirmed, &transcript)
                    .await;
                assert_eq!(score.pass, expected, "{}", score.reason);
                if !expected {
                    assert!(
                        score.reason.contains("before user confirmation"),
                        "{}",
                        score.reason
                    );
                }
            }
        }
    }
}
