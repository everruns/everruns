//! A Subject that runs a model against the in-process control plane.
//!
//! The live subject needs a database, a scheduler and a web server running
//! before a single case executes, which is why these cases were not being run.
//! This one needs a model key and nothing else, so the suite can be part of
//! ordinary work rather than an occasion.
//!
//! It is not a replacement. It fakes persistence, so it cannot measure
//! authorization or anything that depends on a command's real output values;
//! `scheduled_agent_state`, which grades persisted server state, scores N/A
//! here. What it does measure is the thing the command contract is for: whether
//! a model can find a command, spell it, and pass flags that exist. The prompt,
//! the tool schemas, the operation catalog and the argument parser are all the
//! shipped ones, generated from the server.
//!
//! Selected with `EVERRUNS_EVAL_MODE=offline`.

use std::time::Instant;

use mira::{RunCx, Sample, Subject, Transcript};
use serde_json::{Value, json};

use crate::control_plane::{FakeControlPlane, system_prompt, tool_definitions};

/// Ceiling on assistant turns, independent of a sample's own `max_iterations`.
/// A model that never stops calling tools would otherwise burn a key.
const HARD_ITERATION_CAP: usize = 16;

pub struct OfflineSubject {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl OfflineSubject {
    /// Build from environment:
    /// - `OPENROUTER_API_KEY` (required)
    /// - `EVERRUNS_EVAL_OFFLINE_BASE_URL` (default OpenRouter)
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("EVERRUNS_EVAL_OFFLINE_BASE_URL")
                .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string())
                .trim_end_matches('/')
                .to_string(),
            api_key: std::env::var("OPENROUTER_API_KEY").unwrap_or_default(),
            client: reqwest::Client::new(),
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
            .map_err(|e| format!("request failed: {e}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("read body failed: {e}"))?;
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

        let control_plane = FakeControlPlane::new();
        let tools = tool_definitions();
        let mut messages = vec![json!({ "role": "system", "content": system_prompt() })];
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
        if sample.metadata.contains_key("expect_scheduled_agent") {
            return Transcript::infra_error(
                "this case grades persisted server state; run it without EVERRUNS_EVAL_MODE=offline"
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
                    events.push(json!({
                        "type": "tool.started",
                        "data": { "tool_call": { "name": name, "arguments": arguments } }
                    }));

                    let output = control_plane.call(&name, &arguments);
                    tool_calls.push(name.clone());
                    events.push(json!({
                        "type": "tool.completed",
                        "data": { "tool_name": name }
                    }));

                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call.get("id").cloned().unwrap_or(json!("")),
                        "content": output,
                    }));
                }
            }
        }

        transcript.tool_calls_count = tool_calls.len();
        transcript.tool_calls = tool_calls;
        transcript.iterations = iterations;
        transcript.final_response = final_response;
        transcript.events = events;
        transcript.timing.duration_ms = started.elapsed().as_millis() as u64;
        transcript
    }
}
