//! Privacy-safe wait facts retained by the journal, independent of exporters.
use crate::events::{Event, EventData, EventRequest, ToolCompletedData};
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::TurnId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const METADATA_KEY: &str = "everruns.human_waits";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitKind {
    Approval,
    Question,
}

/// No request/result content is retained, even when content tracing is enabled.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HumanWait {
    pub kind: WaitKind,
    pub started_at: DateTime<Utc>,
    pub turn_id: TurnId,
    pub tool_call_id: String,
    pub turn_traceparent: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitOutcome {
    Approved,
    Rejected,
    NotApproved,
    Answered,
    Declined,
    Timeout,
    Cancelled,
    Failed,
    Abandoned,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedWait {
    #[serde(flatten)]
    pub wait: HumanWait,
    pub outcome: WaitOutcome,
}

/// Small per-turn projection. SQLite persists it with the authority state;
/// PostgreSQL reconstructs it from the current request and its resolutions.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HumanWaitState {
    turn: Option<TurnId>,
    anchor: Option<String>,
    pending: BTreeMap<String, HumanWait>,
}

impl HumanWaitState {
    pub fn observe(&mut self, event: &Event) {
        match &event.data {
            EventData::TurnStarted(data) => {
                self.turn = Some(data.turn_id);
                self.pending.clear();
                self.anchor = event
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get(crate::causal::TURN_TRACEPARENT_KEY))
                    .and_then(Value::as_str)
                    .filter(|value| crate::causal::CausalLink::from_traceparent(value).is_some())
                    .map(str::to_owned);
            }
            EventData::ToolCallRequested(data) => {
                let Some(turn) = event.context.turn_id else {
                    return;
                };
                if self.turn != Some(turn) {
                    *self = Self {
                        turn: Some(turn),
                        ..Default::default()
                    };
                }
                for call in data.tool_calls.iter().take(256) {
                    let kind = match call.name.as_str() {
                        everruns_contracts::tool_types::APPROVE_TOOL_CALL_TOOL => {
                            WaitKind::Approval
                        }
                        "ask_user" => WaitKind::Question,
                        _ => continue,
                    };
                    // Matches the command batch cap; duplicates retain the first timestamp.
                    if self.pending.len() < 256 {
                        self.pending
                            .entry(call.id.clone())
                            .or_insert_with(|| HumanWait {
                                kind,
                                started_at: event.ts,
                                turn_id: turn,
                                tool_call_id: call.id.clone(),
                                turn_traceparent: self.anchor.clone(),
                            });
                    }
                }
            }
            EventData::ToolCompleted(data) if event.context.turn_id == self.turn => {
                self.pending.remove(&data.tool_call_id);
            }
            data if terminal_outcome(data).is_some() && event.context.turn_id == self.turn => {
                self.pending.clear();
            }
            _ => {}
        }
    }

    /// Attach facts before commit, using the resolution's authority timestamp.
    /// Recovered claims use their original event, never the retry wall clock.
    pub fn annotate(&self, request: &mut EventRequest) {
        if let Some(metadata) = request.metadata.as_mut().and_then(Value::as_object_mut) {
            metadata.remove(METADATA_KEY);
        }
        if request.context.turn_id != self.turn {
            return;
        }
        let resolved: Vec<_> = match &request.data {
            EventData::ToolCompleted(data) => self
                .pending
                .get(&data.tool_call_id)
                .and_then(|wait| {
                    completion_outcome(wait, data).map(|outcome| ResolvedWait {
                        wait: wait.clone(),
                        outcome,
                    })
                })
                .into_iter()
                .collect(),
            data => terminal_outcome(data)
                .map(|outcome| {
                    self.pending
                        .values()
                        .map(|wait| ResolvedWait {
                            wait: wait.clone(),
                            outcome: outcome.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };
        if !resolved.is_empty() {
            let metadata = request.metadata.get_or_insert_with(|| json!({}));
            if let Some(metadata) = metadata.as_object_mut() {
                metadata.insert(METADATA_KEY.into(), json!(resolved));
            }
        }
    }
}

pub fn may_resolve(data: &EventData) -> bool {
    terminal_outcome(data).is_some()
        || matches!(data, EventData::ToolCompleted(data)
        if data.tool_name == "ask_user" || data.tool_name == everruns_contracts::tool_types::APPROVE_TOOL_CALL_TOOL || data.tool_name.is_empty())
}

fn terminal_outcome(data: &EventData) -> Option<WaitOutcome> {
    match data {
        EventData::TurnCancelled(_) => Some(WaitOutcome::Cancelled),
        EventData::TurnFailed(_) => Some(WaitOutcome::Failed),
        EventData::TurnCompleted(_) | EventData::TurnSealed(_) => Some(WaitOutcome::Abandoned),
        _ => None,
    }
}

fn completion_outcome(wait: &HumanWait, data: &ToolCompletedData) -> Option<WaitOutcome> {
    let result: Option<Value> = data
        .result
        .as_ref()
        .and_then(|parts| parts.first())
        .and_then(|part| part.as_text())
        .and_then(|text| serde_json::from_str(text).ok());
    let result = result.as_ref();
    let field = |name| {
        result
            .and_then(|value| value.get(name))
            .and_then(Value::as_str)
    };
    if matches!(wait.kind, WaitKind::Question) && field("answered_by") == Some("unattended") {
        return None;
    }
    Some(
        if data.status == "timeout" || field("answered_by") == Some("timeout") {
            WaitOutcome::Timeout
        } else if data.status == "cancelled" {
            WaitOutcome::Cancelled
        } else if !data.success {
            WaitOutcome::Failed
        } else {
            match wait.kind {
                WaitKind::Approval => match field("outcome").or_else(|| field("decision")) {
                    Some("allow" | "allow_always") => WaitOutcome::Approved,
                    Some("reject" | "reject_always") => WaitOutcome::Rejected,
                    Some("expired") => WaitOutcome::Timeout,
                    Some("not_approved") => WaitOutcome::NotApproved,
                    _ => WaitOutcome::Unknown,
                },
                WaitKind::Question => match field("status") {
                    Some("answered") => WaitOutcome::Answered,
                    Some("declined") => WaitOutcome::Declined,
                    Some("timeout" | "defaulted") => WaitOutcome::Timeout,
                    _ => WaitOutcome::Unknown,
                },
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventContext, ToolCallRequestedData, TurnCancelledData};
    use crate::message::ContentPart;
    use everruns_contracts::{
        tool_types::ToolCall,
        typed_id::{MessageId, SessionId},
    };

    fn request(
        session: SessionId,
        context: &EventContext,
        data: impl Into<EventData>,
        seconds: i64,
    ) -> EventRequest {
        let mut request = EventRequest::new(session, context.clone(), data);
        request.ts = DateTime::from_timestamp(seconds, 0).unwrap();
        request
    }
    fn committed(request: EventRequest) -> Event {
        request.into_event(everruns_contracts::typed_id::EventId::new(), 1)
    }
    fn fixture() -> (HumanWaitState, SessionId, EventContext) {
        let session = SessionId::new();
        let context = EventContext::turn(TurnId::new(), MessageId::new());
        let calls = [("a", everruns_contracts::tool_types::APPROVE_TOOL_CALL_TOOL), ("q", "ask_user"), ("c", "client_tool")].map(|(id,name)| ToolCall { id: id.into(), name: name.into(), arguments: json!({"prompt":"PRIVATE_PROMPT", "url":"https://private.example/token", "credential":"PRIVATE_KEY"}) });
        let mut state = HumanWaitState::default();
        state.observe(&committed(request(
            session,
            &context,
            ToolCallRequestedData::with_definitions(&calls, &[]),
            100,
        )));
        (state, session, context)
    }
    fn completion(id: &str, result: Value) -> ToolCompletedData {
        ToolCompletedData::success(
            id.into(),
            "".into(),
            vec![ContentPart::tool_result_text(&result)],
            None,
        )
    }
    fn waits(request: &EventRequest) -> Value {
        request
            .metadata
            .as_ref()
            .and_then(|m| m.get(METADATA_KEY))
            .cloned()
            .unwrap_or(json!([]))
    }

    #[test]
    fn human_wait_outcomes_do_not_depend_on_tool_success() {
        for (id, result, expected) in [
            ("a", json!({"outcome":"reject"}), "rejected"),
            ("a", json!({"decision":"allow_always"}), "approved"),
            ("a", json!({"outcome":"expired"}), "timeout"),
            ("a", json!({"outcome":"not_approved"}), "not_approved"),
            (
                "q",
                json!({"status":"declined","answered_by":"timeout"}),
                "timeout",
            ),
            (
                "q",
                json!({"status":"declined","answered_by":"user"}),
                "declined",
            ),
            (
                "q",
                json!({"status":"answered","answered_by":"user","answers":["PRIVATE_ANSWER"]}),
                "answered",
            ),
            ("a", json!({"decision":"PRIVATE_UNKNOWN"}), "unknown"),
        ] {
            let (state, session, context) = fixture();
            let mut result = request(session, &context, completion(id, result), 130);
            state.annotate(&mut result);
            assert_eq!(waits(&result)[0]["outcome"], expected);
            let metadata = serde_json::to_string(&result.metadata).unwrap();
            assert!(!metadata.contains("PRIVATE"));
            assert!(!metadata.contains("https://"));
        }
    }

    #[test]
    fn human_wait_partial_batch_restart_duplicates_and_terminal() {
        let (mut state, session, context) = fixture();
        let mut answer = request(
            session,
            &context,
            completion("q", json!({"status":"answered"})),
            130,
        );
        state.annotate(&mut answer);
        assert_eq!(waits(&answer).as_array().unwrap().len(), 1);
        state.observe(&committed(answer.clone()));
        let mut state: HumanWaitState =
            serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
        state.annotate(&mut answer);
        assert_eq!(waits(&answer), json!([]));
        let mut cancel = request(
            session,
            &context,
            TurnCancelledData {
                turn_id: context.turn_id.unwrap(),
                reason: Some("PRIVATE_REASON".into()),
                usage: None,
            },
            150,
        );
        state.annotate(&mut cancel);
        assert_eq!(waits(&cancel).as_array().unwrap().len(), 1);
        assert_eq!(waits(&cancel)[0]["tool_call_id"], "a");
        assert_eq!(waits(&cancel)[0]["outcome"], "cancelled");
        assert_eq!(waits(&cancel)[0]["started_at"], "1970-01-01T00:01:40Z");
        state.observe(&committed(cancel));
        let mut late = request(
            session,
            &context,
            completion("a", json!({"outcome":"allow"})),
            160,
        );
        state.annotate(&mut late);
        assert_eq!(waits(&late), json!([]));
    }

    #[test]
    fn human_wait_unattended_missing_history_and_other_turn_do_not_invent_waits() {
        let (state, session, context) = fixture();
        let mut unattended = request(
            session,
            &context,
            completion(
                "q",
                json!({"status":"defaulted", "answered_by":"unattended"}),
            ),
            100,
        );
        state.annotate(&mut unattended);
        assert_eq!(waits(&unattended), json!([]));
        let mut unknown = request(
            session,
            &context,
            completion("q", json!({"status":"answered"})),
            110,
        );
        HumanWaitState::default().annotate(&mut unknown);
        assert_eq!(waits(&unknown), json!([]));
        unknown.context.turn_id = Some(TurnId::new());
        state.annotate(&mut unknown);
        assert_eq!(waits(&unknown), json!([]));
    }
}
