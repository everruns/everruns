//! `#[eval]`: conversations with assertions, run in-process or against a URL.
//!
//! Decision: the same eval code runs against the local binary (fast, offline
//! with the simulator) and, with `--against <url>`, against a deployed build
//! over the wire API. That makes an eval run a promotion gate for a preview
//! deploy without a second test harness.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use everruns::ask_user::{AskUser, DefaultsResponder, Question, Status};
use serde_json::{Value, json};

use crate::app::{App, Mode};
use crate::host::{Host, NewSession, Notice, wire_json};

const TURN_TIMEOUT: Duration = Duration::from_secs(180);
const POLL: Duration = Duration::from_millis(100);

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

/// One finished turn, as the eval saw it.
#[derive(Clone, Debug, Default)]
pub struct TurnRecord {
    pub response: String,
    pub success: bool,
    pub error: Option<String>,
    /// Tools called, in order.
    pub tools: Vec<String>,
    /// Tools that asked for approval.
    pub approvals: Vec<String>,
    /// `ask_user` question sets answered (with their declared defaults).
    pub questions: usize,
    /// The turn's durable canonical events, as the wire API sends them.
    pub events: Vec<Value>,
}

/// The eval's handle on one session.
pub struct EvalCx {
    target: Target,
    session: Option<String>,
    agent: Option<String>,
    /// Last durable sequence seen; the next turn's events come after it.
    cursor: i32,
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

    #[cfg(test)]
    pub(crate) fn remote_for_test(base: String) -> Self {
        Self::new(Target::Remote {
            base,
            client: reqwest::Client::new(),
        })
    }

    /// Talk to this agent instead of the default one. Call before `send`.
    pub fn agent(&mut self, name: impl Into<String>) -> &mut Self {
        self.agent = Some(name.into());
        self
    }

    /// How approval requests are answered (default: approve). Questions from
    /// `ask_user` are answered with their declared defaults.
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
        if let Some(last) = turn
            .events
            .iter()
            .filter_map(|e| e["sequence"].as_i64())
            .max()
        {
            self.cursor = i32::try_from(last).unwrap_or(self.cursor);
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
                host.create_session(NewSession {
                    agent: self.agent.clone(),
                    metadata: Some(json!({ "eval": true })),
                    ..NewSession::default()
                })
                .await
            }
            Target::Remote { base, client } => {
                let body: Value = client
                    .post(format!("{base}/v1/sessions"))
                    .json(&json!({ "agent_name": self.agent, "metadata": { "eval": true } }))
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

    /// In-process: send, answer approvals and questions as the host parks
    /// them, and take the outcome from the turn itself.
    async fn local_turn(
        &self,
        host: Arc<Host>,
        session: &str,
        text: String,
    ) -> crate::Result<TurnRecord> {
        let mut notices = host.notices.subscribe();
        let pending = host.send(session, text).await?;
        let mut record = TurnRecord::default();
        let wait = pending.wait();
        tokio::pin!(wait);
        let deadline = tokio::time::sleep(TURN_TIMEOUT);
        tokio::pin!(deadline);
        let outcome = loop {
            tokio::select! {
                outcome = &mut wait => break outcome?,
                () = &mut deadline => bail!("turn did not finish within {TURN_TIMEOUT:?}"),
                notice = notices.recv() => match notice {
                    Ok(Notice::ApprovalRequested(view)) if view.session_id == session => {
                        record.approvals.push(view.tool_name.clone());
                        // The turn may have moved on (cancel) in between.
                        let _ = host.resolve_approval(
                            session,
                            &view.tool_call_id,
                            self.on_approval == OnApproval::Approve,
                        );
                    }
                    Ok(Notice::QuestionAsked { session_id, tool_call_id, questions })
                        if session_id == session =>
                    {
                        record.questions += 1;
                        let outcome = DefaultsResponder.ask(&questions).await;
                        let _ = host.answer_questions(
                            session,
                            Some(&tool_call_id),
                            Status::Answered,
                            outcome.answers,
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        bail!("host shut down mid-turn")
                    }
                    _ => {}
                },
            }
        };
        record.response = outcome.response;
        record.success = outcome.success;
        record.error = outcome.error;
        record.events = host
            .events_after(session, self.cursor)
            .await?
            .iter()
            .map(wire_json)
            .collect();
        record.tools = tools_called(&record.events);
        Ok(record)
    }

    /// Over the wire: send, then poll the session, answering its pending
    /// approvals and questions, until it is idle and the turn's terminal
    /// event is in the log.
    async fn remote_turn(&self, session: &str, text: String) -> crate::Result<TurnRecord> {
        let Target::Remote { base, client } = &self.target else {
            bail!("not a remote eval");
        };
        client
            .post(format!("{base}/v1/sessions/{session}/messages"))
            .json(&json!({ "message": { "role": "user", "content": [{ "type": "text", "text": text }] } }))
            .send()
            .await?
            .error_for_status()?;
        let mut record = TurnRecord::default();
        let deadline = Instant::now() + TURN_TIMEOUT;
        loop {
            if Instant::now() > deadline {
                bail!("turn did not finish within {TURN_TIMEOUT:?}");
            }
            let state: Value = client
                .get(format!("{base}/v1/sessions/{session}"))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            for pending in state["pending_approvals"].as_array().into_iter().flatten() {
                record.approvals.push(
                    pending["tool_name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                );
                let decision = match self.on_approval {
                    OnApproval::Approve => "approve",
                    OnApproval::Deny => "deny",
                };
                let call = pending["tool_call_id"].as_str().unwrap_or_default();
                client
                    .post(format!("{base}/v1/sessions/{session}/approvals/{call}"))
                    .json(&json!({ "decision": decision, "note": "answered by eval" }))
                    .send()
                    .await?;
            }
            for pending in state["pending_questions"].as_array().into_iter().flatten() {
                record.questions += 1;
                let questions: Vec<Question> =
                    serde_json::from_value(pending["questions"].clone())?;
                let outcome = DefaultsResponder.ask(&questions).await;
                client
                    .post(format!("{base}/v1/sessions/{session}/question-answers"))
                    .json(&json!({
                        "tool_call_id": pending["tool_call_id"],
                        "status": "answered",
                        "answers": outcome.answers,
                    }))
                    .send()
                    .await?;
            }
            if state["status"] == "idle" {
                let events: Value = client
                    .get(format!(
                        "{base}/v1/sessions/{session}/events?after_sequence={}",
                        self.cursor
                    ))
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                let events = events["data"].as_array().cloned().unwrap_or_default();
                if let Some(terminal) = events.iter().rev().find(|event| is_terminal(event)) {
                    record.success = terminal["type"] == "turn.completed";
                    record.error = terminal["data"]["error"].as_str().map(str::to_string);
                    record.response = final_response(&events);
                    record.tools = tools_called(&events);
                    record.events = events;
                    return Ok(record);
                }
            }
            tokio::time::sleep(POLL).await;
        }
    }
}

fn is_terminal(event: &Value) -> bool {
    matches!(
        event["type"].as_str(),
        Some("turn.completed" | "turn.failed" | "turn.cancelled")
    )
}

/// Tool names from canonical `tool.started` events, in order.
fn tools_called(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event["type"] == "tool.started")
        .filter_map(|event| event["data"]["tool_call"]["name"].as_str())
        .map(str::to_string)
        .collect()
}

/// The text of the turn's last completed output message that has any.
fn final_response(events: &[Value]) -> String {
    events
        .iter()
        .rev()
        .filter(|event| event["type"] == "output.message.completed")
        .map(|event| {
            event["data"]["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|part| part["type"] == "text")
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .find(|text| !text.is_empty())
        .unwrap_or_default()
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

    #[test]
    fn remote_turns_read_tools_and_the_reply_from_canonical_events() {
        let events = vec![
            json!({ "type": "tool.started", "data": { "tool_call": { "name": "run_sql" } } }),
            json!({ "type": "output.message.completed", "data": { "message": { "content": [{ "type": "text", "text": "Net of refunds." }] } } }),
            json!({ "type": "output.message.completed", "data": { "message": { "content": [] } } }),
            json!({ "type": "turn.completed", "data": {} }),
        ];
        assert_eq!(tools_called(&events), vec!["run_sql"]);
        assert_eq!(final_response(&events), "Net of refunds.");
        assert!(is_terminal(&events[3]));
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
}
