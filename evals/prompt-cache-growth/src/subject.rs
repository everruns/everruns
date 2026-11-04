//! Drives a running Everruns server over HTTP: one session per sample, the
//! sample's turns in order, then the session's events back into a transcript.
//!
//! The server, not an in-process Framework agent, because the cache breaks this
//! study exists for came from the hosted path: the worker loads history from
//! the server, and an in-process run never takes that route.

use std::time::{Duration, Instant};

use mira::{ErrorKind, RunCx, Sample, Subject, Transcript};
use serde_json::{Value, json};

use crate::cache::{Cause, Report};
use crate::everruns_usage::summarize_events;

pub const CALLS_KEY: &str = "cache.calls";
pub const BREAKS_KEY: &str = "cache.breaks_detail";

const TERMINAL: &[&str] = &[
    "turn.completed",
    "turn.failed",
    "turn.cancelled",
    "turn.sealed",
    "session.failed",
];

pub struct ServerSubject {
    base_url: String,
    api_key: String,
    agent: String,
    turn_timeout: Duration,
    client: reqwest::Client,
}

impl ServerSubject {
    /// - `EVERRUNS_API_URL` (default `http://localhost:9300/api`)
    /// - `EVERRUNS_API_KEY` (default `dev`; `Bearer evr_pat_...` for a PAT)
    /// - `EVERRUNS_EVAL_AGENT` (default `platform-chat`)
    /// - `EVERRUNS_EVAL_TURN_TIMEOUT_SECS` (default 300)
    pub fn from_env() -> Self {
        let env = |name: &str, default: &str| std::env::var(name).unwrap_or(default.to_string());
        Self {
            base_url: env("EVERRUNS_API_URL", "http://localhost:9300/api")
                .trim_end_matches('/')
                .to_string(),
            api_key: env("EVERRUNS_API_KEY", "dev"),
            agent: env("EVERRUNS_EVAL_AGENT", "platform-chat"),
            turn_timeout: Duration::from_secs(
                env("EVERRUNS_EVAL_TURN_TIMEOUT_SECS", "300")
                    .parse()
                    .unwrap_or(300),
            ),
            client: reqwest::Client::new(),
        }
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let mut request = self
            .client
            .request(method.clone(), format!("{}{path}", self.base_url))
            .header("Authorization", &self.api_key);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| format!("{method} {path}: {e}"))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("{method} {path}: HTTP {status}: {text}"));
        }
        serde_json::from_str(&text).map_err(|e| format!("{method} {path}: bad JSON: {e}"))
    }

    /// `model_…` ids pass through; anything else is looked up by `model_id`
    /// or display name. An empty model keeps the agent's default.
    async fn model_id(&self, model: &str) -> Result<Option<String>, String> {
        if model.is_empty() || model.starts_with("model_") {
            return Ok((!model.is_empty()).then(|| model.to_string()));
        }
        let models = self
            .request(reqwest::Method::GET, "/v1/models", None)
            .await?;
        models["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|m| m["model_id"] == model || m["display_name"] == model)
            .and_then(|m| m["id"].as_str())
            .map(|id| Some(id.to_string()))
            .ok_or_else(|| format!("unknown model '{model}' on the server"))
    }

    /// Every event after `cursor`, following pages.
    async fn events_after(&self, session: &str, mut cursor: i64) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        loop {
            let page = self
                .request(
                    reqwest::Method::GET,
                    &format!("/v1/sessions/{session}/events?after_sequence={cursor}&limit=500"),
                    None,
                )
                .await?;
            let events = page["data"].as_array().cloned().unwrap_or_default();
            if events.is_empty() {
                return Ok(out);
            }
            cursor = events
                .iter()
                .filter_map(|e| e["sequence"].as_i64())
                .max()
                .unwrap_or(cursor);
            out.extend(events);
        }
    }

    async fn play(&self, sample: &Sample, cx: &RunCx) -> Result<(String, Option<String>), String> {
        let mut body = json!({
            "agent_name": self.agent,
            "title": format!("Mira eval: {}", sample.id),
            "tags": ["mira", "prompt-cache-growth"],
        });
        if let Some(model) = self.model_id(cx.target.model.trim()).await? {
            body["model_id"] = Value::String(model);
        }
        let session = self
            .request(reqwest::Method::POST, "/v1/sessions", Some(body))
            .await?;
        let session = session["id"]
            .as_str()
            .ok_or("create session: no id")?
            .to_string();

        let mut cursor = 0;
        for turn in &sample.input {
            let message = json!({ "message": { "content": [{ "type": "text", "text": turn }] } });
            if let Err(error) = self
                .request(
                    reqwest::Method::POST,
                    &format!("/v1/sessions/{session}/messages"),
                    Some(message),
                )
                .await
            {
                return Ok((session, Some(error)));
            }
            let deadline = Instant::now() + self.turn_timeout;
            loop {
                let events = self.events_after(&session, cursor).await?;
                cursor = events
                    .iter()
                    .filter_map(|e| e["sequence"].as_i64())
                    .max()
                    .unwrap_or(cursor);
                if events
                    .iter()
                    .any(|e| TERMINAL.contains(&e["type"].as_str().unwrap_or_default()))
                {
                    break;
                }
                if Instant::now() >= deadline {
                    let _ = self
                        .request(
                            reqwest::Method::POST,
                            &format!("/v1/sessions/{session}/cancel"),
                            Some(json!({})),
                        )
                        .await;
                    return Ok((
                        session,
                        Some(format!(
                            "turn timed out after {}s",
                            self.turn_timeout.as_secs()
                        )),
                    ));
                }
                tokio::time::sleep(Duration::from_millis(750)).await;
            }
        }
        Ok((session, None))
    }
}

#[async_trait::async_trait]
impl Subject for ServerSubject {
    async fn run(&self, sample: &Sample, cx: &RunCx) -> Transcript {
        let started = Instant::now();
        let (session, turn_error) = match self.play(sample, cx).await {
            Ok(played) => played,
            Err(error) => return Transcript::infra_error(error),
        };
        let events = match self.events_after(&session, 0).await {
            Ok(events) => events,
            Err(error) => return Transcript::infra_error(format!("read events: {error}")),
        };
        let mut transcript = Transcript::default();
        if let Some(error) = turn_error.or_else(|| terminal_error(&events)) {
            transcript.error_kind = classify(&error);
            transcript.error = Some(error);
        }
        transcript.final_response = final_response(&events).unwrap_or_default();
        summarize_events(&events).apply_to(&mut transcript);
        record(&mut transcript, &Report::from_events(&events));
        transcript
            .metadata
            .insert("session_id".to_string(), json!(session));
        transcript.timing.duration_ms = started.elapsed().as_millis() as u64;
        transcript
    }
}

/// Cache metrics and the per-call table go on the transcript, so scorers and
/// the runs report read the same numbers.
pub fn record(transcript: &mut Transcript, report: &Report) {
    if let Some(rate) = report.hit_rate() {
        transcript.record_metric("cache.hit_rate", rate);
    }
    if let Some(rate) = report.late_hit_rate() {
        transcript.record_metric("cache.hit_rate_late", rate);
    }
    transcript.record_metric("cache.breaks", report.breaks().count() as f64);
    for cause in Cause::ALL {
        transcript.record_metric(
            format!("cache.breaks.{}", cause.as_str()),
            report.count(cause) as f64,
        );
    }
    transcript
        .metadata
        .insert(CALLS_KEY.to_string(), report.to_json());
    transcript
        .metadata
        .insert(BREAKS_KEY.to_string(), json!(report.describe_breaks()));
}

fn final_response(events: &[Value]) -> Option<String> {
    let message = events
        .iter()
        .rfind(|e| e["type"] == "output.message.completed")?;
    let parts = message.pointer("/data/message/content")?.as_array()?;
    Some(
        parts
            .iter()
            .filter(|p| p["type"] == "text")
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn terminal_error(events: &[Value]) -> Option<String> {
    events
        .iter()
        .find(|e| matches!(e["type"].as_str(), Some("turn.failed" | "session.failed")))
        .map(|e| {
            e.pointer("/data/error")
                .map(|error| match error {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| e["type"].as_str().unwrap_or_default().to_string())
        })
}

/// Rate limits, outages and timeouts are infra (N/A, retried); anything else is
/// the subject's.
fn classify(message: &str) -> ErrorKind {
    let lower = message.to_ascii_lowercase();
    const INFRA: &[&str] = &[
        "429",
        "402",
        "500",
        "502",
        "503",
        "rate limit",
        "quota",
        "credit",
        "timed out",
        "timeout",
        "connection",
    ];
    if INFRA.iter().any(|signal| lower.contains(signal)) {
        ErrorKind::Infra
    } else {
        ErrorKind::Subject
    }
}
