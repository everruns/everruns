//! A Mira `Subject` that drives a running everruns server's `platform-chat`
//! session — the live-runtime path to evaluating the real `platform_management`
//! capability end to end.
//!
//! For each sample it creates a session on the `platform-chat` harness (which
//! carries `platform_management`), plays the sample's turns, waits for each turn
//! to complete, then reads the session event stream back into a Mira
//! `Transcript` (final response + tool calls + token usage). Scoring and
//! reporting are shared with every other Mira subject.
//!
//! Why HTTP and not `mira-everruns`' in-process `RuntimeSubject`: the
//! `platform_management` tools require a DB-backed `PlatformStore` and a session
//! runner, which only the full server provides. Driving the server exercises the
//! real tools against real persistence — the thing we actually want to measure.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use mira::subject::summarize_events;
use mira::{ErrorKind, RunCx, Sample, Subject, Transcript};
use serde_json::{Value, json};
use tokio::sync::OnceCell;

/// Default harness that carries the `platform_management` capability.
const DEFAULT_HARNESS: &str = "platform-chat";

pub struct EverrunsServerSubject {
    base_url: String,
    api_key: String,
    harness: String,
    poll_interval: Duration,
    turn_timeout: Duration,
    client: reqwest::Client,
    /// Lazily-fetched map from friendly model name (`model_id` or display name)
    /// to the server's prefixed model id (`model_…`).
    model_cache: OnceCell<HashMap<String, String>>,
}

impl EverrunsServerSubject {
    /// Build from environment:
    /// - `EVERRUNS_API_URL` (default `http://localhost:9300/api`)
    /// - `EVERRUNS_API_KEY` (default `dev`; use `Bearer evr_pat_...` for a PAT)
    /// - `EVERRUNS_EVAL_HARNESS` (default `platform-chat`)
    /// - `EVERRUNS_EVAL_TURN_TIMEOUT_SECS` (default 180)
    pub fn from_env() -> Self {
        let base_url = std::env::var("EVERRUNS_API_URL")
            .unwrap_or_else(|_| "http://localhost:9300/api".to_string())
            .trim_end_matches('/')
            .to_string();
        let api_key = std::env::var("EVERRUNS_API_KEY").unwrap_or_else(|_| "dev".to_string());
        let harness =
            std::env::var("EVERRUNS_EVAL_HARNESS").unwrap_or_else(|_| DEFAULT_HARNESS.to_string());
        let turn_timeout = std::env::var("EVERRUNS_EVAL_TURN_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(180);
        Self {
            base_url,
            api_key,
            harness,
            poll_interval: Duration::from_millis(750),
            turn_timeout: Duration::from_secs(turn_timeout),
            client: reqwest::Client::new(),
            model_cache: OnceCell::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        let resp = self
            .client
            .post(self.url(path))
            .header("Authorization", &self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("POST {path}: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("POST {path}: HTTP {status}: {text}"));
        }
        serde_json::from_str(&text).map_err(|e| format!("POST {path}: bad JSON: {e}"))
    }

    async fn get(&self, path: &str) -> Result<Value, String> {
        let resp = self
            .client
            .get(self.url(path))
            .header("Authorization", &self.api_key)
            .send()
            .await
            .map_err(|e| format!("GET {path}: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("GET {path}: HTTP {status}: {text}"));
        }
        serde_json::from_str(&text).map_err(|e| format!("GET {path}: bad JSON: {e}"))
    }

    /// Map the matrix target onto the server's prefixed `model_id`. A `sim`/empty
    /// target means "use the session's default model" (no override). A value that
    /// already looks like a prefixed id (`model_…`) is used verbatim; anything
    /// else is resolved against `/v1/models` by `model_id` or display name.
    async fn resolve_model_id(&self, cx: &RunCx) -> Result<Option<String>, String> {
        let model = cx.target.model.trim();
        if model.is_empty() || cx.target.provider == "sim" || model == "sim" {
            return Ok(None);
        }
        if model.starts_with("model_") {
            return Ok(Some(model.to_string()));
        }
        let map = self.model_map().await?;
        map.get(model).cloned().map(Some).ok_or_else(|| {
            format!("unknown model '{model}': not a 'model_' id, model_id, or display name on the server")
        })
    }

    async fn model_map(&self) -> Result<&HashMap<String, String>, String> {
        self.model_cache
            .get_or_try_init(|| async {
                let resp = self.get("/v1/models").await?;
                let mut map = HashMap::new();
                if let Some(items) = resp.get("data").and_then(|d| d.as_array()) {
                    for it in items {
                        let Some(pid) = it.get("id").and_then(|v| v.as_str()) else {
                            continue;
                        };
                        if let Some(mid) = it.get("model_id").and_then(|v| v.as_str()) {
                            map.insert(mid.to_string(), pid.to_string());
                        }
                        if let Some(dn) = it.get("display_name").and_then(|v| v.as_str()) {
                            map.entry(dn.to_string()).or_insert_with(|| pid.to_string());
                        }
                    }
                }
                Ok(map)
            })
            .await
    }

    async fn create_session(&self, sample: &Sample, cx: &RunCx) -> Result<String, String> {
        let mut body = json!({
            "harness_name": self.harness,
            "title": format!("Mira eval: {}", sample.id),
            "tags": ["mira", "platform-capability"],
        });
        if let Some(model) = self.resolve_model_id(cx).await? {
            body["model_id"] = Value::String(model);
        }
        let resp = self.post("/v1/sessions", body).await?;
        resp.get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| format!("create session: no id in response: {resp}"))
    }

    async fn send_message(&self, session_id: &str, text: &str) -> Result<(), String> {
        let body = json!({ "message": { "content": [{ "type": "text", "text": text }] } });
        self.post(&format!("/v1/sessions/{session_id}/messages"), body)
            .await
            .map(|_| ())
    }

    /// Fetch events after `cursor`; returns (events, new_cursor).
    async fn events_after(
        &self,
        session_id: &str,
        cursor: i64,
    ) -> Result<(Vec<Value>, i64), String> {
        let path = if cursor > 0 {
            format!("/v1/sessions/{session_id}/events?after_sequence={cursor}")
        } else {
            format!("/v1/sessions/{session_id}/events")
        };
        let resp = self.get(&path).await?;
        let events = resp
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        let mut max = cursor;
        for e in &events {
            if let Some(seq) = e.get("sequence").and_then(|s| s.as_i64()) {
                max = max.max(seq);
            }
        }
        Ok((events, max))
    }

    /// After a message is sent, poll until the turn finishes (a `turn.completed`
    /// / `turn.failed` / `turn.cancelled` or `session.failed` event appears).
    async fn wait_for_turn(&self, session_id: &str, cursor: i64) -> Result<i64, String> {
        const TERMINAL: &[&str] = &[
            "turn.completed",
            "turn.failed",
            "turn.cancelled",
            "turn.sealed",
            "session.failed",
        ];
        let deadline = Instant::now() + self.turn_timeout;
        let mut cur = cursor;
        loop {
            let (events, new_cur) = self.events_after(session_id, cur).await?;
            cur = new_cur;
            if events
                .iter()
                .filter_map(|e| e.get("type").and_then(|t| t.as_str()))
                .any(|t| TERMINAL.contains(&t))
            {
                return Ok(cur);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "turn did not complete within {}s",
                    self.turn_timeout.as_secs()
                ));
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }
}

#[async_trait::async_trait]
impl Subject for EverrunsServerSubject {
    async fn run(&self, sample: &Sample, cx: &RunCx) -> Transcript {
        let started = Instant::now();

        let session_id = match self.create_session(sample, cx).await {
            Ok(id) => id,
            // Building the session is scaffolding — attribute to infra so the
            // case is scored N/A (skipped) rather than failed, and retried.
            Err(e) => return Transcript::infra_error(format!("create session failed: {e}")),
        };

        // Establish a baseline cursor so we only wait on events for our turns.
        let mut cursor = match self.events_after(&session_id, 0).await {
            Ok((_, c)) => c,
            Err(e) => return Transcript::infra_error(format!("read baseline events: {e}")),
        };

        let mut transcript = Transcript::default();
        for turn in &sample.input {
            if let Err(e) = self.send_message(&session_id, turn).await {
                transcript.error = Some(e.clone());
                transcript.error_kind = classify(&e);
                break;
            }
            match self.wait_for_turn(&session_id, cursor).await {
                Ok(c) => cursor = c,
                Err(e) => {
                    transcript.error = Some(e.clone());
                    transcript.error_kind = classify(&e);
                    break;
                }
            }
        }

        // Pull the full event stream and normalize into the transcript.
        match self.events_after(&session_id, 0).await {
            Ok((events, _)) => {
                transcript.tool_calls = extract_tool_calls(&events);
                transcript.tool_calls_count = transcript.tool_calls.len();
                transcript.iterations = count_turns(&events);
                if let Some(final_text) = extract_final_response(&events) {
                    transcript.final_response = final_text;
                }
                let (usage, _tools) = summarize_events(&events);
                transcript.usage = usage;
                transcript.events = events;
            }
            Err(e) => {
                if transcript.error.is_none() {
                    transcript.error = Some(format!("read events failed: {e}"));
                    transcript.error_kind = classify(&e);
                }
            }
        }

        transcript.timing.duration_ms = started.elapsed().as_millis() as u64;
        transcript
    }
}

/// Tool names, in call order, from `tool.completed` events.
fn extract_tool_calls(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("tool.completed"))
        .filter_map(|e| {
            e.get("data")
                .and_then(|d| d.get("tool_name"))
                .and_then(|n| n.as_str())
                .map(String::from)
        })
        .collect()
}

/// Final assistant text from the last `output.message.completed` event.
/// Event data shape: `{ "message": { "content": [{ "type": "text", "text": ... }] } }`.
fn extract_final_response(events: &[Value]) -> Option<String> {
    events
        .iter()
        .rfind(|e| e.get("type").and_then(|t| t.as_str()) == Some("output.message.completed"))
        .and_then(|e| e.get("data"))
        .and_then(|d| d.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
}

fn count_turns(events: &[Value]) -> usize {
    events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("turn.completed"))
        .count()
}

/// Bucket transport/timeout faults as infra (scored N/A, retried) and leave the
/// rest as a genuine, scoreable subject failure.
fn classify(message: &str) -> ErrorKind {
    let m = message.to_ascii_lowercase();
    const INFRA: &[&str] = &[
        "did not complete within",
        "timed out",
        "timeout",
        "connection refused",
        "connection reset",
        "connection closed",
        "network",
        "503",
        "502",
        "500",
        "bad gateway",
        "gateway timeout",
    ];
    if INFRA.iter().any(|s| m.contains(s)) {
        ErrorKind::Infra
    } else {
        ErrorKind::Subject
    }
}
