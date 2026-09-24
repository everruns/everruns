//! `#[eval]`: conversations with assertions, run in-process or against a URL.
//!
//! Decision: the same eval code runs against the local binary (fast, offline
//! with the simulator) and, with `--against <url>`, against a deployed build
//! over the wire API. That makes an eval run a promotion gate for a preview
//! deploy without a second test harness.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use futures::StreamExt;
use serde_json::{Value, json};

use crate::app::{App, Mode};
use crate::host::Host;
use crate::store::WireEvent;

const TURN_TIMEOUT: Duration = Duration::from_secs(180);

enum Target {
    Local(Arc<Host>),
    Remote {
        base: String,
        client: reqwest::Client,
    },
}

/// What to do when a tool asks for approval during an eval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnApproval {
    Approve,
    Deny,
}

/// One finished turn, as the eval saw it on the wire.
#[derive(Clone, Debug, Default)]
pub struct TurnRecord {
    pub response: String,
    pub success: bool,
    pub error: Option<String>,
    /// Tools called, in order.
    pub tools: Vec<String>,
    /// Tools that asked for approval.
    pub approvals: Vec<String>,
    pub events: Vec<WireEvent>,
}

/// The eval's handle on one session.
pub struct EvalCx {
    target: Target,
    session: Option<String>,
    agent: Option<String>,
    cursor: i64,
    on_approval: OnApproval,
    turns: Vec<TurnRecord>,
}

impl EvalCx {
    fn new(target: Target) -> Self {
        Self {
            target,
            session: None,
            agent: None,
            cursor: 0,
            on_approval: OnApproval::Approve,
            turns: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn local_for_test(host: Arc<Host>) -> Self {
        Self::new(Target::Local(host))
    }

    /// Talk to this agent instead of the default one. Call before `send`.
    pub fn agent(&mut self, name: impl Into<String>) -> &mut Self {
        self.agent = Some(name.into());
        self
    }

    /// How approval requests are answered (default: approve).
    pub fn on_approval(&mut self, policy: OnApproval) -> &mut Self {
        self.on_approval = policy;
        self
    }

    /// Send a message and wait for the turn to finish.
    pub async fn send(&mut self, text: impl Into<String>) -> crate::Result {
        let text = text.into();
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                let session = self.create().await?;
                self.session = Some(session.clone());
                session
            }
        };
        let turn = match &self.target {
            Target::Local(host) => self.local_turn(host.clone(), &session, text).await?,
            Target::Remote { .. } => self.remote_turn(&session, text).await?,
        };
        if let Some(last) = turn.events.last() {
            self.cursor = last.seq;
        }
        self.turns.push(turn);
        Ok(())
    }

    /// The last turn, asserting it completed successfully.
    pub fn completed(&self) -> crate::Result<TurnCheck<'_>> {
        let turn = self.last()?;
        if !turn.success {
            bail!(
                "turn did not complete: {}",
                turn.error.clone().unwrap_or_default()
            );
        }
        Ok(TurnCheck { turn })
    }

    /// The last turn, whatever its outcome.
    pub fn last(&self) -> crate::Result<&TurnRecord> {
        self.turns
            .last()
            .ok_or_else(|| anyhow!("no turn yet; call send() first"))
    }

    async fn create(&self) -> crate::Result<String> {
        match &self.target {
            Target::Local(host) => {
                host.create_session(self.agent.as_deref(), json!({ "eval": true }), None)
                    .await
            }
            Target::Remote { base, client } => {
                let url = match &self.agent {
                    Some(agent) => format!("{base}/v1/agents/{agent}/sessions"),
                    None => format!("{base}/v1/sessions"),
                };
                let body: Value = client
                    .post(url)
                    .json(&json!({ "metadata": { "eval": true } }))
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                body.get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| anyhow!("create session returned no id: {body}"))
            }
        }
    }

    async fn local_turn(
        &self,
        host: Arc<Host>,
        session: &str,
        text: String,
    ) -> crate::Result<TurnRecord> {
        let mut live = host.events.subscribe();
        host.send(session, text).await?;
        let mut turn = Turn::after(self.cursor);
        let deadline = Instant::now() + TURN_TIMEOUT;
        for event in host.events_after(session, self.cursor)? {
            self.observe_local(&host, &mut turn, event)?;
        }
        while !turn.done() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = tokio::time::timeout(remaining, live.recv())
                .await
                .map_err(|_| anyhow!("turn did not finish within {TURN_TIMEOUT:?}"))?;
            match event {
                Ok(event) if event.session_id == session => {
                    self.observe_local(&host, &mut turn, event)?
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    for event in host.events_after(session, turn.last)? {
                        self.observe_local(&host, &mut turn, event)?;
                    }
                }
                Err(err) => return Err(err.into()),
            }
        }
        Ok(turn.record)
    }

    fn observe_local(&self, host: &Host, turn: &mut Turn, event: WireEvent) -> crate::Result {
        if let Some(approval) = turn.observe(event) {
            host.resolve_approval(
                &approval.session,
                &approval.id,
                self.on_approval == OnApproval::Approve,
                Some("answered by eval".into()),
            )?;
        }
        Ok(())
    }

    async fn remote_turn(&self, session: &str, text: String) -> crate::Result<TurnRecord> {
        let Target::Remote { base, client } = &self.target else {
            bail!("not a remote eval");
        };
        // Open the stream first so nothing the turn emits is missed.
        let response = client
            .get(format!("{base}/v1/sessions/{session}/events"))
            .header("last-event-id", self.cursor.to_string())
            .send()
            .await?
            .error_for_status()?;
        client
            .post(format!("{base}/v1/sessions/{session}/messages"))
            .json(&json!({ "input": text }))
            .send()
            .await?
            .error_for_status()?;

        let mut turn = Turn::after(self.cursor);
        let mut bytes = response.bytes_stream();
        let mut buffer = String::new();
        let deadline = Instant::now() + TURN_TIMEOUT;
        while !turn.done() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let chunk = tokio::time::timeout(remaining, bytes.next())
                .await
                .map_err(|_| anyhow!("turn did not finish within {TURN_TIMEOUT:?}"))?
                .ok_or_else(|| anyhow!("event stream closed mid-turn"))??;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(end) = buffer.find("\n\n") {
                let block: String = buffer.drain(..end + 2).collect();
                let Some(event) = parse_sse(&block) else {
                    continue;
                };
                if let Some(approval) = turn.observe(event) {
                    let decision = match self.on_approval {
                        OnApproval::Approve => "approve",
                        OnApproval::Deny => "deny",
                    };
                    client
                        .post(format!(
                            "{base}/v1/sessions/{}/approvals/{}",
                            approval.session, approval.id
                        ))
                        .json(&json!({ "decision": decision, "note": "answered by eval" }))
                        .send()
                        .await?
                        .error_for_status()?;
                }
            }
        }
        Ok(turn.record)
    }
}

struct PendingApproval {
    session: String,
    id: String,
}

/// Accumulates one turn's events until both the runtime's terminal event and
/// the host's `turn.result` have been seen, which guarantees every event of the
/// turn is in the record.
#[derive(Default)]
struct Turn {
    record: TurnRecord,
    /// Highest sequence observed; replay and live delivery can overlap.
    last: i64,
    terminal: bool,
    result: bool,
}

impl Turn {
    fn done(&self) -> bool {
        self.terminal && self.result
    }

    fn after(cursor: i64) -> Self {
        Self {
            last: cursor,
            ..Self::default()
        }
    }

    fn observe(&mut self, event: WireEvent) -> Option<PendingApproval> {
        if event.seq <= self.last {
            return None;
        }
        self.last = event.seq;
        let mut approval = None;
        match event.kind.as_str() {
            "tool.started" => {
                if let Some(name) = event.data.get("tool_name").and_then(Value::as_str) {
                    self.record.tools.push(name.to_string());
                }
            }
            "approval.requested" => {
                let tool = event
                    .data
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.record.approvals.push(tool.to_string());
                if let Some(id) = event.data.get("approval_id").and_then(Value::as_str) {
                    approval = Some(PendingApproval {
                        session: event.session_id.clone(),
                        id: id.to_string(),
                    });
                }
            }
            "turn.completed" | "turn.failed" | "turn.cancelled" => self.terminal = true,
            "turn.result" => {
                self.result = true;
                self.record.response = str_field(&event.data, "response");
                self.record.success =
                    event.data.get("success").and_then(Value::as_bool) == Some(true);
                self.record.error = event
                    .data
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            _ => {}
        }
        self.record.events.push(event);
        approval
    }
}

fn str_field(data: &Value, key: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn parse_sse(block: &str) -> Option<WireEvent> {
    let data: String = block
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::from_str(&data).ok()
}

/// Assertions on a completed turn. Each returns `Result<Self>` so they chain
/// with `?`.
pub struct TurnCheck<'a> {
    turn: &'a TurnRecord,
}

impl<'a> TurnCheck<'a> {
    pub fn called_tool(self, name: &str) -> crate::Result<Self> {
        if self.turn.tools.iter().any(|tool| tool == name) {
            Ok(self)
        } else {
            bail!(
                "expected a `{name}` call; tools called: {:?}",
                self.turn.tools
            )
        }
    }

    pub fn did_not_call(self, name: &str) -> crate::Result<Self> {
        if self.turn.tools.iter().any(|tool| tool == name) {
            bail!("`{name}` was called but should not have been")
        }
        Ok(self)
    }

    pub fn asked_approval(self, tool: &str) -> crate::Result<Self> {
        if self.turn.approvals.iter().any(|name| name == tool) {
            Ok(self)
        } else {
            bail!(
                "expected `{tool}` to ask for approval; approvals: {:?}",
                self.turn.approvals
            )
        }
    }

    /// Case-insensitive substring match on the reply.
    pub fn reply_contains(self, needle: &str) -> crate::Result<Self> {
        if self
            .turn
            .response
            .to_lowercase()
            .contains(&needle.to_lowercase())
        {
            Ok(self)
        } else {
            bail!(
                "reply does not contain {needle:?}: {:?}",
                self.turn.response
            )
        }
    }

    pub fn reply(&self) -> &'a str {
        &self.turn.response
    }
}

/// Outcome of an eval run.
#[derive(Clone, Debug, Default)]
pub struct EvalReport {
    pub results: Vec<EvalResult>,
}

#[derive(Clone, Debug)]
pub struct EvalResult {
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
    pub duration: Duration,
}

impl EvalReport {
    pub fn passed(&self) -> bool {
        self.results.iter().all(|result| result.passed)
    }
}

/// Run the app's evals, optionally only those whose name contains `filter`.
pub(crate) async fn run(
    app: &App,
    against: Option<&str>,
    filter: Option<&str>,
) -> crate::Result<EvalReport> {
    let local = match against {
        Some(_) => None,
        None => Some(Host::new(app.clone(), Mode::Eval, None)?),
    };
    let mut report = EvalReport::default();
    for eval in &app.inner.evals {
        if filter.is_some_and(|filter| !eval.name.contains(filter)) {
            continue;
        }
        let target = match (&local, against) {
            (Some(host), _) => Target::Local(host.clone()),
            (None, Some(base)) => Target::Remote {
                base: base.trim_end_matches('/').to_string(),
                client: reqwest::Client::new(),
            },
            (None, None) => bail!("no eval target"),
        };
        let mut cx = EvalCx::new(target);
        let started = Instant::now();
        let outcome = (eval.run)(&mut cx).await;
        let result = EvalResult {
            name: eval.name.to_string(),
            passed: outcome.is_ok(),
            error: outcome.err().map(|err| format!("{err:#}")),
            duration: started.elapsed(),
        };
        match &result.error {
            None => println!("  ✓ {} ({:.1?})", result.name, result.duration),
            Some(err) => println!("  ✗ {} ({:.1?})\n      {err}", result.name, result.duration),
        }
        report.results.push(result);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(seq: i64, kind: &str, data: Value) -> WireEvent {
        WireEvent {
            seq,
            session_id: "s".into(),
            kind: kind.into(),
            at: String::new(),
            data,
        }
    }

    #[test]
    fn a_turn_is_done_only_after_both_terminal_events() {
        let mut turn = Turn::default();
        turn.observe(event(1, "tool.started", json!({ "tool_name": "run_sql" })));
        turn.observe(event(
            2,
            "turn.result",
            json!({ "response": "ok", "success": true }),
        ));
        assert!(!turn.done());
        turn.observe(event(3, "turn.completed", json!({})));
        assert!(turn.done());
        assert_eq!(turn.record.tools, vec!["run_sql"]);
        assert!(turn.record.success);
    }

    #[test]
    fn approvals_are_surfaced_for_answering() {
        let mut turn = Turn::default();
        let pending = turn
            .observe(event(
                1,
                "approval.requested",
                json!({ "approval_id": "apr_1", "tool": "run_sql" }),
            ))
            .unwrap();
        assert_eq!(pending.id, "apr_1");
        assert_eq!(turn.record.approvals, vec!["run_sql"]);
    }

    #[test]
    fn checks_explain_failures() {
        let record = TurnRecord {
            response: "Revenue was $10, net of refunds.".into(),
            success: true,
            tools: vec!["run_sql".into()],
            ..TurnRecord::default()
        };
        let check = TurnCheck { turn: &record };
        let check = check
            .called_tool("run_sql")
            .unwrap()
            .reply_contains("NET OF REFUNDS")
            .unwrap();
        let err = check
            .called_tool("delete_everything")
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("run_sql"), "{err}");
    }

    #[test]
    fn sse_blocks_parse_to_wire_events() {
        let block = "id: 3\nevent: turn.result\ndata: {\"seq\":3,\"session_id\":\"s\",\"type\":\"turn.result\",\"at\":\"\",\"data\":{}}\n\n";
        let event = parse_sse(block).unwrap();
        assert_eq!(event.seq, 3);
        assert_eq!(event.kind, "turn.result");
        assert!(parse_sse(": keep-alive\n\n").is_none());
    }
}
