//! A Mira `Subject` that drives a running everruns server's `platform-chat`
//! session — the live-runtime path to evaluating the real `platform`
//! capability end to end.
//!
//! For each sample it creates a session on the `platform-chat` harness (which
//! carries `platform`), plays the sample's turns, waits for each turn
//! to complete, then reads the session event stream back into a Mira
//! `Transcript` (final response + tool calls + token usage). Scoring and
//! reporting are shared with every other Mira subject.
//!
//! Why HTTP and not `mira-everruns`' in-process `RuntimeSubject`: the
//! `platform` tools require a DB-backed `PlatformStore` and a session
//! runner, which only the full server provides. Driving the server exercises the
//! real tools against real persistence — the thing we actually want to measure.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::everruns_usage::summarize_events;
use mira::{ErrorKind, RunCx, Sample, Subject, Transcript};
use serde_json::{Value, json};
use tokio::sync::OnceCell;

/// Canonical managed Agent; other configured names select custom harnesses.
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
        // The canonical operator moved from a harness to a managed Agent.
        // Never silently qualify the retired harness instead of what ships.
        if let Some(agent) = sample
            .metadata
            .get("agent_name")
            .and_then(Value::as_str)
            .or_else(|| (self.harness == DEFAULT_HARNESS).then_some(DEFAULT_HARNESS))
        {
            body = json!({
                "agent_name": agent,
                "title": format!("Mira eval: {}", sample.id),
                "tags": ["mira", "platform-capability"],
            });
        }
        if let Some(hints) = sample.metadata.get("hints") {
            body["hints"] = hints.clone();
        }
        if let Some(model) = self.resolve_model_id(cx).await? {
            body["model_id"] = Value::String(model);
        }
        let resp = self.post("/v1/sessions", body).await?;
        resp.get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| format!("create session: no id in response: {resp}"))
    }

    async fn capture_scheduled_agent_state(&self, session_id: &str, resource_name: &str) -> Value {
        let agent = match self.get(&format!("/v1/agents/{resource_name}")).await {
            Ok(agent) => agent,
            Err(error) => return json!({ "error": format!("read created agent: {error}") }),
        };
        let agent_id = agent.get("id").and_then(Value::as_str);
        let model_id = agent.get("default_model_id").and_then(Value::as_str);

        let model = match model_id {
            Some(id) => self.get(&format!("/v1/models/{id}")).await.unwrap_or_else(
                |error| json!({ "error": format!("read selected model: {error}") }),
            ),
            None => json!({ "error": "created agent has no default_model_id" }),
        };
        let triggers = match agent_id {
            Some(id) => self
                .get(&format!("/v1/agents/{id}/triggers"))
                .await
                .unwrap_or_else(
                    |error| json!({ "error": format!("read agent triggers: {error}") }),
                ),
            None => json!({ "error": "created agent has no id" }),
        };
        let session_schedules = self
            .get(&format!("/v1/sessions/{session_id}/schedules"))
            .await
            .unwrap_or_else(
                |error| json!({ "error": format!("read Platform Chat schedules: {error}") }),
            );
        let mcp_servers = self
            .get(&format!("/v1/mcp-servers?search={resource_name}-visti"))
            .await
            .unwrap_or_else(|error| json!({ "error": format!("read Visti MCP server: {error}") }));
        let credentials = match agent_id {
            Some(id) => self
                .get(&format!("/v1/agents/{id}/credentials"))
                .await
                .unwrap_or_else(
                    |error| json!({ "error": format!("read Agent credentials: {error}") }),
                ),
            None => json!({ "error": "created agent has no id" }),
        };

        json!({
            "agent": agent,
            "model": model,
            "triggers": triggers,
            "session_schedules": session_schedules,
            "mcp_servers": mcp_servers,
            "credentials": credentials,
        })
    }

    /// Register the catalog MCP server a case adds from, if it is not there.
    async fn ensure_catalog_mcp(&self, server: &Value) -> Result<(), String> {
        match self.post("/v1/mcp-servers", server.clone()).await {
            Ok(_) => Ok(()),
            Err(error) if error.contains("409") || error.contains("already exists") => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Approve every tool-approval request the turn parked on, as the person
    /// would by pressing Allow on the card. True when there was one.
    async fn approve_pending(&self, session_id: &str, events: &[Value]) -> Result<bool, String> {
        let decisions: Vec<Value> = events
            .iter()
            .filter(|event| {
                event.get("type").and_then(Value::as_str) == Some("tool.call_requested")
            })
            .filter_map(|event| event.pointer("/data/tool_calls").and_then(Value::as_array))
            .flatten()
            .filter(|call| call.get("name").and_then(Value::as_str) == Some("approve_tool_call"))
            .filter_map(|call| call.get("id").and_then(Value::as_str))
            .map(|id| json!({ "tool_call_id": id, "decision": "allow" }))
            .collect();
        if decisions.is_empty() {
            return Ok(false);
        }
        self.post(
            &format!("/v1/sessions/{session_id}/tool-approvals"),
            json!({ "decisions": decisions }),
        )
        .await?;
        Ok(true)
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
    async fn wait_for_turn(
        &self,
        session_id: &str,
        cursor: i64,
        // (tool calls, iterations) seen so far in this sample.
        seen: &mut (usize, usize),
        max_tool_calls: Option<usize>,
        max_iterations: Option<usize>,
        stop_on_pause: bool,
    ) -> Result<i64, String> {
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
            seen.0 += events
                .iter()
                .filter(|event| event.get("type").and_then(Value::as_str) == Some("tool.started"))
                .count();
            seen.1 += events
                .iter()
                .filter(|event| event.get("type").and_then(Value::as_str) == Some("reason.started"))
                .count();
            let budget_error = budget_error(seen.0, seen.1, max_tool_calls, max_iterations);
            if let Some(error) = budget_error {
                let _ = self
                    .post(&format!("/v1/sessions/{session_id}/cancel"), json!({}))
                    .await;
                return Err(error);
            }
            if events
                .iter()
                .filter_map(|e| e.get("type").and_then(|t| t.as_str()))
                .any(|t| TERMINAL.contains(&t) || (stop_on_pause && t == "tool.call_requested"))
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
        if sample
            .metadata
            .get("surface_comparison")
            .and_then(Value::as_bool)
            == Some(true)
        {
            return Transcript::infra_error("surface-comparison fixtures require EVERRUNS_EVAL_MODE=offline; matched fixture surfaces are offline-only; hosted Platform Chat uses its managed capability configuration".to_string());
        }
        if std::env::var("EVERRUNS_EVAL_SURFACE").is_ok() {
            return Transcript::infra_error("EVERRUNS_EVAL_SURFACE selects an offline comparison; set EVERRUNS_EVAL_MODE=offline".to_string());
        }
        let started = Instant::now();

        if let Some(server) = sample.metadata.get("ensure_catalog_mcp")
            && let Err(e) = self.ensure_catalog_mcp(server).await
        {
            return Transcript::infra_error(format!("register catalog MCP server: {e}"));
        }
        // A turn that parks on a card (tool approval, Connect) ends here
        // instead of timing out; approvals are answered when the case asks.
        let stop_on_pause = sample
            .metadata
            .get("approve_tool_calls")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || sample.metadata.contains_key("hints");
        let approve = sample
            .metadata
            .get("approve_tool_calls")
            .and_then(Value::as_bool)
            .unwrap_or(false);

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
        let max_tool_calls = sample
            .metadata
            .get("max_tool_calls")
            .and_then(Value::as_u64)
            .map(|value| value as usize);
        let max_iterations = sample
            .metadata
            .get("max_iterations")
            .and_then(Value::as_u64)
            .map(|value| value as usize);
        let mut seen = (0usize, 0usize);
        let resource_name = sample
            .metadata
            .get("resource_name_prefix")
            .and_then(Value::as_str)
            .map(unique_resource_name);
        if let Some(name) = &resource_name {
            transcript
                .metadata
                .insert("resource_name".to_string(), json!(name));
        }
        for turn in &sample.input {
            let materialized = resource_name.as_ref().map_or_else(
                || turn.clone(),
                |name| turn.replace("{{resource_name}}", name),
            );
            if let Err(e) = self.send_message(&session_id, &materialized).await {
                transcript.error = Some(e.clone());
                transcript.error_kind = classify(&e);
                break;
            }
            // Bounded: one turn may park on approvals a few times.
            let mut failed = None;
            for _ in 0..4 {
                let before = cursor;
                match self
                    .wait_for_turn(
                        &session_id,
                        cursor,
                        &mut seen,
                        max_tool_calls,
                        max_iterations,
                        stop_on_pause,
                    )
                    .await
                {
                    Ok(c) => cursor = c,
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
                if !approve {
                    break;
                }
                let parked = match self.events_after(&session_id, before).await {
                    Ok((events, _)) => self.approve_pending(&session_id, &events).await,
                    Err(e) => Err(e),
                };
                match parked {
                    Ok(true) => continue,
                    Ok(false) => break,
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            }
            if let Some(e) = failed {
                transcript.error = Some(e.clone());
                transcript.error_kind = classify(&e);
                break;
            }
        }

        // Pull the full event stream and normalize into the transcript.
        match self.events_after(&session_id, 0).await {
            Ok((events, _)) => {
                transcript.tool_calls = extract_tool_calls(&events);
                transcript.tool_calls_count = transcript.tool_calls.len();
                transcript.iterations = count_iterations(&events);
                if let Some(final_text) = extract_final_response(&events) {
                    transcript.final_response = final_text;
                }
                summarize_events(&events).apply_to(&mut transcript);
                if transcript.error.is_none()
                    && let Some(error) = extract_terminal_error(&events)
                {
                    transcript.error_kind = classify(&error);
                    transcript.error = Some(error);
                }
                transcript.events = events;
                crate::scorers::record_friction(&mut transcript);
            }
            Err(e) => {
                if transcript.error.is_none() {
                    transcript.error = Some(format!("read events failed: {e}"));
                    transcript.error_kind = classify(&e);
                }
            }
        }

        if sample.metadata.contains_key("expect_scheduled_agent")
            && let Some(name) = &resource_name
        {
            let state = self.capture_scheduled_agent_state(&session_id, name).await;
            transcript
                .metadata
                .insert("platform_state".to_string(), state);
        }

        transcript.timing.duration_ms = started.elapsed().as_millis() as u64;
        transcript
    }
}

fn unique_resource_name(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{:x}-{}", nanos & 0xffffffffff, std::process::id())
}

fn budget_error(
    tool_calls: usize,
    iterations: usize,
    max_tool_calls: Option<usize>,
    max_iterations: Option<usize>,
) -> Option<String> {
    max_tool_calls
        .filter(|max| tool_calls > *max)
        .map(|max| format!("tool call budget exceeded: {tool_calls}/{max}"))
        .or_else(|| {
            max_iterations
                .filter(|max| iterations > *max)
                .map(|max| format!("iteration budget exceeded: {iterations}/{max}"))
        })
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

fn count_iterations(events: &[Value]) -> usize {
    events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("reason.started"))
        .count()
}

fn extract_terminal_error(events: &[Value]) -> Option<String> {
    events.iter().rev().find_map(|event| {
        matches!(
            event.get("type").and_then(Value::as_str),
            Some("turn.failed" | "session.failed")
        )
        .then(|| {
            event
                .pointer("/data/error_fields/detail")
                .or_else(|| event.pointer("/data/error"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .flatten()
    })
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
        "credit",
        "quota",
        "rate limit",
        "429",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hosted_session_creation_targets_the_managed_agent_and_preserves_overrides() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut buffer = [0; 4096];
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        break position + 4;
                    }
                };
                let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
                assert!(headers.starts_with("POST /v1/sessions "));
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|length| length.trim().parse().unwrap())
                    })
                    .unwrap();
                while bytes.len() < header_end + length {
                    let mut buffer = [0; 4096];
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                }
                requests.push(
                    serde_json::from_slice::<Value>(&bytes[header_end..header_end + length])
                        .unwrap(),
                );
                let response = r#"{"id":"session_mock"}"#;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
            }
            requests
        });
        let mut subject = EverrunsServerSubject {
            base_url: format!("http://{address}"),
            api_key: "test".into(),
            harness: DEFAULT_HARNESS.into(),
            poll_interval: Duration::from_millis(1),
            turn_timeout: Duration::from_secs(5),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            model_cache: OnceCell::new(),
        };
        let mut sample = mira::Dataset::from_jsonl_str(crate::DATASET)
            .unwrap()
            .samples
            .remove(0);
        let cx = RunCx::new(mira::Target::new("default", "everruns", ""));
        for (harness, agent) in [
            (DEFAULT_HARNESS, None),
            ("custom-harness", None),
            ("custom-harness", Some("assigned-agent")),
        ] {
            subject.harness = harness.into();
            sample.metadata.remove("agent_name");
            if let Some(agent) = agent {
                sample.metadata.insert("agent_name".into(), json!(agent));
            }
            assert_eq!(
                subject.create_session(&sample, &cx).await.unwrap(),
                "session_mock"
            );
        }
        let requests = server.join().unwrap();
        assert_eq!(requests[0]["agent_name"], "platform-chat");
        assert!(requests[0].get("harness_name").is_none());
        assert_eq!(requests[1]["harness_name"], "custom-harness");
        assert!(requests[1].get("agent_name").is_none());
        assert_eq!(requests[2]["agent_name"], "assigned-agent");
        assert!(requests[2].get("harness_name").is_none());
    }

    #[test]
    fn budget_is_inclusive_and_reports_first_exceeded_limit() {
        assert_eq!(budget_error(12, 20, Some(12), Some(20)), None);
        assert_eq!(
            budget_error(13, 21, Some(12), Some(20)).as_deref(),
            Some("tool call budget exceeded: 13/12")
        );
        assert_eq!(
            budget_error(2, 21, Some(12), Some(20)).as_deref(),
            Some("iteration budget exceeded: 21/20")
        );
    }

    #[test]
    fn terminal_provider_failure_is_extracted_and_classified_as_infra() {
        let events = vec![json!({
            "type": "turn.failed",
            "data": {
                "error": "processing error",
                "error_fields": {"detail": "credit_balance_exhausted: no credits remaining"}
            }
        })];
        let error = extract_terminal_error(&events).unwrap();
        assert_eq!(classify(&error), ErrorKind::Infra);
    }
}
