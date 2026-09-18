//! Slack approvals: the card, the click, and who may click it (EVE-1025).
//!
//! # No new protocol
//!
//! The `soft_approval` capability already pauses a turn: `request_approval` is
//! a real tool call, the model ends its turn on it, and the pause is answered by
//! *the user's next message*. Slack does not need the synthetic-tool-call
//! variant of pause-and-consent that `setup_connection` and `url_elicitation`
//! use, because there is nothing to pause — the turn is already over.
//!
//! So a Slack approval is two small pieces bolted onto machinery that exists:
//!
//! 1. When a `request_approval` call completes in a Slack thread, the delivery
//!    adapter renders it as Block Kit buttons instead of leaving the thread
//!    holding a bare question.
//! 2. A click becomes the next user message on that session. The turn resumes
//!    exactly as it would have if the person had typed "yes".
//!
//! That second point is what keeps attribution honest for free.
//! [`crate::services::approval_audit`] resolves the approver from the
//! `input.message` event the API wrote, never from anything the model said. A
//! click posts a real message from a real Slack identity, so the audit row names
//! the person who clicked without approvals needing their own identity path.
//!
//! # Who may click
//!
//! The interactivity endpoint is unauthenticated in the same sense the events
//! endpoint is: Slack signs it, nothing else does. So a click must bind to
//! **both** the pending request and an identified Slack user, or anyone who can
//! see the channel can approve consequential work.
//!
//! The binding is carried in the button's `value`, which Slack echoes back
//! unchanged and covers with the same signature as the rest of the request. A
//! viewer cannot edit another person's button, and a forged POST does not
//! verify. The requester is stamped into the card at render time, so the check
//! at click time is an equality test rather than a lookup that could race the
//! session's state.
//!
//! The default is the requester alone, because it is the only policy that is
//! never surprising. [`ApprovalPolicy`] is where an allowlist or any-member
//! setting attaches.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Tool whose completion raises a Slack approval card.
pub(crate) const REQUEST_APPROVAL_TOOL: &str = "request_approval";

/// Session hint a Slack endpoint declares when it can draw approval cards.
///
/// Deliberately its own key rather than reusing `setup_connection`: a surface
/// that renders one card does not necessarily render another, and Client Hints
/// states the cost of assuming otherwise — "pausing a turn on a card nobody
/// draws just burns the tool-result timeout."
pub const SLACK_APPROVAL_HINT: &str = "slack_approval";

/// Slack caps `action_id` and `value` at 255 and 2000 bytes.
///
/// The value carries the binding a click is authorized against, so it must not
/// be silently truncated into something that still parses. Anything over the
/// cap renders as a plain question instead of a card.
const MAX_ACTION_VALUE_BYTES: usize = 2000;

pub(crate) const APPROVE_ACTION_ID: &str = "everruns_approve";
pub(crate) const DECLINE_ACTION_ID: &str = "everruns_decline";

/// What the agent stopped in front of, read back out of its `tool.completed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovalRequest {
    pub action: String,
    pub question: Option<String>,
}

/// The binding a click is checked against, round-tripped through Slack.
///
/// Carried in the button's `value`. Slack echoes it back unchanged and signs
/// the request that carries it, so this is trustworthy exactly as far as the
/// signature check is — which is the same trust the events endpoint already
/// runs on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ApprovalBinding {
    /// Session the pending request belongs to.
    #[serde(rename = "s")]
    pub session_id: String,
    /// Slack user who asked, and the only one who may answer by default.
    #[serde(rename = "u")]
    pub requester: String,
    /// The turn the ask was raised in, as `request_approval` reported it.
    ///
    /// Carried so a click on a card from an earlier turn is recognisable as
    /// stale rather than answering whatever is pending now.
    #[serde(rename = "t", skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// What the card asked about.
    ///
    /// Carried rather than re-derived at click time, so the decision recorded
    /// names the action that was actually on screen. Re-reading it from the
    /// session could answer a *different* pause than the one clicked.
    #[serde(rename = "a")]
    pub action: String,
}

/// Who may answer an approval card.
///
/// Only the default is implemented. The other two are the shape the per-channel
/// setting will take, named here so the decision is visible rather than implied
/// by a missing branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum ApprovalPolicy {
    /// Only the Slack user whose message raised the request.
    #[default]
    Requester,
}

impl ApprovalPolicy {
    /// Whether `clicker` may answer a card bound to `binding`.
    pub fn allows(&self, binding: &ApprovalBinding, clicker: &str) -> bool {
        match self {
            // An empty requester would make the comparison vacuous and let
            // anyone in, so it refuses instead: a card we could not bind is a
            // card nobody may answer.
            Self::Requester => !binding.requester.is_empty() && binding.requester == clicker,
        }
    }

    /// Why a refusal happened, phrased for the person who clicked.
    pub fn refusal(&self, binding: &ApprovalBinding) -> String {
        match self {
            Self::Requester if binding.requester.is_empty() => {
                "This request cannot be approved from Slack because it is not bound to a requester."
                    .to_string()
            }
            Self::Requester => format!("Only <@{}> can answer this request.", binding.requester),
        }
    }
}

/// A decision a click carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApprovalDecision {
    Approved,
    Declined,
}

impl ApprovalDecision {
    pub fn from_action_id(action_id: &str) -> Option<Self> {
        match action_id {
            APPROVE_ACTION_ID => Some(Self::Approved),
            DECLINE_ACTION_ID => Some(Self::Declined),
            _ => None,
        }
    }

    /// The message posted into the session on the agent's behalf.
    ///
    /// Plain language on purpose. It becomes an ordinary user message, and the
    /// model reads it the same way it reads a typed "yes" — the whole point of
    /// answering through the existing path rather than a bespoke one.
    pub fn as_message(self, action: &str) -> String {
        match self {
            Self::Approved => format!("Approved: {action}"),
            Self::Declined => format!("Declined: {action}"),
        }
    }

    /// How the card reads once it has been answered.
    pub fn as_resolution(self, clicker: &str) -> String {
        match self {
            Self::Approved => format!("Approved by <@{clicker}>"),
            Self::Declined => format!("Declined by <@{clicker}>"),
        }
    }
}

/// Whether a session's hints say it can render an approval card.
///
/// Absent or false is the documented degradation, not a gap: the ask still
/// reaches the thread as prose and the human answers in prose, which is exactly
/// today's behaviour.
pub(crate) fn approvals_enabled(hints: Option<&Value>) -> bool {
    hint_is_set(hints.and_then(|hints| hints.get(SLACK_APPROVAL_HINT)))
}

/// Same question, asked of the decoded hint map the session API carries.
///
/// Two entry points rather than one because the hints reach this crate in two
/// shapes — a raw JSONB column and a decoded map — and converting one to the
/// other just to ask a boolean would allocate on every registration.
pub(crate) fn approvals_enabled_in(
    hints: Option<&std::collections::HashMap<String, Value>>,
) -> bool {
    hint_is_set(hints.and_then(|hints| hints.get(SLACK_APPROVAL_HINT)))
}

fn hint_is_set(hint: Option<&Value>) -> bool {
    hint.and_then(Value::as_bool).unwrap_or(false)
}

/// Read an approval request out of a `tool.completed` event's data.
///
/// Returns `None` for every other tool, for a failed call, and for a
/// `request_approval` result that does not actually say it is waiting — the
/// context-free `execute` path returns the same shape without raising a pause,
/// and rendering a card for it would leave a button nothing answers.
pub(crate) fn extract_approval_request(data: &Value) -> Option<ApprovalRequest> {
    if data.get("tool_name")?.as_str()? != REQUEST_APPROVAL_TOOL {
        return None;
    }
    if !data.get("success")?.as_bool().unwrap_or(false) {
        return None;
    }

    let text = data.get("result")?.as_array()?.iter().find_map(|part| {
        match part.get("type")?.as_str()? {
            "text" => part.get("text")?.as_str(),
            _ => None,
        }
    })?;
    let payload: Value = serde_json::from_str(text).ok()?;

    if !payload
        .get("awaiting_approval")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }

    let action = payload.get("action")?.as_str()?.trim();
    if action.is_empty() {
        return None;
    }
    Some(ApprovalRequest {
        action: action.to_string(),
        question: payload
            .get("question")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    })
}

/// The turn a `request_approval` result says it was raised in.
pub(crate) fn approval_turn_id(data: &Value) -> Option<String> {
    let text = data.get("result")?.as_array()?.iter().find_map(|part| {
        match part.get("type")?.as_str()? {
            "text" => part.get("text")?.as_str(),
            _ => None,
        }
    })?;
    let payload: Value = serde_json::from_str(text).ok()?;
    payload
        .get("asked_in_turn")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Build the Block Kit blocks for an approval card.
///
/// Returns `None` when the binding will not fit in a button value, which is the
/// one case where a card would render buttons that cannot be authorized. The
/// caller falls back to posting the question as text, which is exactly the
/// no-hint behaviour: the model asks, the human answers in prose.
pub(crate) fn build_approval_blocks(
    request: &ApprovalRequest,
    binding: &ApprovalBinding,
) -> Option<Value> {
    let value = serde_json::to_string(binding).ok()?;
    if value.len() > MAX_ACTION_VALUE_BYTES {
        return None;
    }

    let mut text = format!("*Approval needed*\n{}", request.action);
    if let Some(question) = &request.question {
        text.push_str("\n\n");
        text.push_str(question);
    }

    Some(json!([
        {
            "type": "section",
            "text": { "type": "mrkdwn", "text": text }
        },
        {
            "type": "actions",
            "elements": [
                {
                    "type": "button",
                    "action_id": APPROVE_ACTION_ID,
                    "style": "primary",
                    "text": { "type": "plain_text", "text": "Approve" },
                    "value": value,
                },
                {
                    "type": "button",
                    "action_id": DECLINE_ACTION_ID,
                    "style": "danger",
                    "text": { "type": "plain_text", "text": "Decline" },
                    "value": value,
                }
            ]
        }
    ]))
}

/// The card's fallback/notification text, and what it degrades to without a hint.
pub(crate) fn approval_fallback_text(request: &ApprovalRequest) -> String {
    match &request.question {
        Some(question) => format!("Approval needed: {}\n{}", request.action, question),
        None => format!("Approval needed: {}", request.action),
    }
}

/// Replacement blocks for a card that has been answered.
///
/// Answering rewrites the message rather than posting beside it, which is what
/// makes a second click harmless: the buttons are gone, so there is nothing
/// left to click, and the thread reads as a decision rather than as a question
/// with an answer somewhere below it.
pub(crate) fn build_resolved_blocks(request: &ApprovalRequest, resolution: &str) -> Value {
    json!([
        {
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": format!("*Approval needed*\n{}", request.action)
            }
        },
        {
            "type": "context",
            "elements": [{ "type": "mrkdwn", "text": resolution }]
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completed(tool: &str, success: bool, payload: Value) -> Value {
        json!({
            "tool_name": tool,
            "success": success,
            "result": [{ "type": "text", "text": payload.to_string() }],
        })
    }

    fn waiting_payload() -> Value {
        json!({
            "ok": true,
            "awaiting_approval": true,
            "action": "delete the staging database",
            "question": "This drops 12k rows. Go ahead?",
            "asked_in_turn": "turn_1",
        })
    }

    #[test]
    fn an_awaiting_request_approval_is_a_card() {
        let request =
            extract_approval_request(&completed(REQUEST_APPROVAL_TOOL, true, waiting_payload()))
                .expect("a waiting approval is a card");
        assert_eq!(request.action, "delete the staging database");
        assert_eq!(
            request.question.as_deref(),
            Some("This drops 12k rows. Go ahead?")
        );
    }

    #[test]
    fn another_tool_is_not_a_card() {
        assert!(
            extract_approval_request(&completed("sql_query", true, waiting_payload())).is_none()
        );
    }

    #[test]
    fn a_failed_call_is_not_a_card() {
        assert!(
            extract_approval_request(&completed(REQUEST_APPROVAL_TOOL, false, waiting_payload()))
                .is_none()
        );
    }

    /// The context-free `execute` path returns the same shape without raising a
    /// pause. A card for it would be a button nothing answers.
    #[test]
    fn a_result_that_is_not_waiting_is_not_a_card() {
        let payload = json!({
            "ok": true,
            "awaiting_approval": false,
            "action": "delete the staging database",
        });
        assert!(
            extract_approval_request(&completed(REQUEST_APPROVAL_TOOL, true, payload)).is_none()
        );
    }

    #[test]
    fn an_empty_action_is_not_a_card() {
        let payload = json!({ "ok": true, "awaiting_approval": true, "action": "   " });
        assert!(
            extract_approval_request(&completed(REQUEST_APPROVAL_TOOL, true, payload)).is_none()
        );
    }

    #[test]
    fn the_turn_is_carried_through() {
        assert_eq!(
            approval_turn_id(&completed(REQUEST_APPROVAL_TOOL, true, waiting_payload())).as_deref(),
            Some("turn_1")
        );
    }

    fn binding() -> ApprovalBinding {
        ApprovalBinding {
            session_id: "session_1".to_string(),
            requester: "U_REQUESTER".to_string(),
            turn_id: Some("turn_1".to_string()),
            action: "delete the staging database".to_string(),
        }
    }

    #[test]
    fn the_requester_may_answer() {
        assert!(ApprovalPolicy::Requester.allows(&binding(), "U_REQUESTER"));
    }

    /// The core authorization guarantee: seeing the channel is not approving.
    #[test]
    fn a_bystander_may_not_answer() {
        assert!(!ApprovalPolicy::Requester.allows(&binding(), "U_SOMEONE_ELSE"));
        let refusal = ApprovalPolicy::Requester.refusal(&binding());
        assert!(
            refusal.contains("U_REQUESTER"),
            "the refusal must say who can answer: {refusal}"
        );
    }

    /// An unbound card must fail closed rather than let the empty string match
    /// an empty clicker id.
    #[test]
    fn an_unbound_card_may_not_be_answered_by_anyone() {
        let unbound = ApprovalBinding {
            requester: String::new(),
            ..binding()
        };
        assert!(!ApprovalPolicy::Requester.allows(&unbound, ""));
        assert!(!ApprovalPolicy::Requester.allows(&unbound, "U_ANYONE"));
    }

    #[test]
    fn the_binding_round_trips_through_a_button_value() {
        let blocks = build_approval_blocks(
            &ApprovalRequest {
                action: "delete the staging database".to_string(),
                question: None,
            },
            &binding(),
        )
        .expect("a small binding fits");

        let value = blocks[1]["elements"][0]["value"]
            .as_str()
            .expect("the button carries a value");
        let decoded: ApprovalBinding =
            serde_json::from_str(value).expect("the value is the binding");
        assert_eq!(decoded, binding());
    }

    /// A binding that will not fit cannot be authorized on the way back, so no
    /// card is drawn and the caller degrades to prose.
    #[test]
    fn an_oversized_binding_draws_no_card() {
        let oversized = ApprovalBinding {
            session_id: "s".repeat(MAX_ACTION_VALUE_BYTES + 1),
            ..binding()
        };
        assert!(
            build_approval_blocks(
                &ApprovalRequest {
                    action: "x".to_string(),
                    question: None
                },
                &oversized,
            )
            .is_none()
        );
    }

    #[test]
    fn both_buttons_are_offered_and_map_to_decisions() {
        let blocks = build_approval_blocks(
            &ApprovalRequest {
                action: "x".to_string(),
                question: None,
            },
            &binding(),
        )
        .expect("blocks");
        let elements = blocks[1]["elements"].as_array().expect("two buttons");
        assert_eq!(elements.len(), 2);

        assert_eq!(
            ApprovalDecision::from_action_id(elements[0]["action_id"].as_str().unwrap()),
            Some(ApprovalDecision::Approved)
        );
        assert_eq!(
            ApprovalDecision::from_action_id(elements[1]["action_id"].as_str().unwrap()),
            Some(ApprovalDecision::Declined)
        );
        assert_eq!(ApprovalDecision::from_action_id("something_else"), None);
    }

    /// A decision becomes an ordinary user message, because that is what the
    /// existing pause is answered by.
    #[test]
    fn a_decision_reads_as_a_plain_answer() {
        assert_eq!(
            ApprovalDecision::Approved.as_message("delete the staging database"),
            "Approved: delete the staging database"
        );
        assert_eq!(
            ApprovalDecision::Declined.as_message("delete the staging database"),
            "Declined: delete the staging database"
        );
    }

    /// The answered card keeps the ask and drops the buttons, so a second click
    /// has nothing to hit.
    #[test]
    fn an_answered_card_has_no_buttons_left() {
        let request = ApprovalRequest {
            action: "delete the staging database".to_string(),
            question: Some("Go ahead?".to_string()),
        };
        let resolved =
            build_resolved_blocks(&request, &ApprovalDecision::Approved.as_resolution("U_R"));
        let rendered = resolved.to_string();
        assert!(rendered.contains("delete the staging database"));
        assert!(rendered.contains("Approved by <@U_R>"));
        assert!(
            !rendered.contains(APPROVE_ACTION_ID) && !rendered.contains(DECLINE_ACTION_ID),
            "an answered card must not keep its buttons: {rendered}"
        );
    }

    #[test]
    fn the_hint_gates_the_card() {
        assert!(approvals_enabled(Some(
            &json!({ SLACK_APPROVAL_HINT: true })
        )));
        assert!(!approvals_enabled(Some(
            &json!({ SLACK_APPROVAL_HINT: false })
        )));
        assert!(!approvals_enabled(Some(
            &json!({ "setup_connection": true })
        )));
        assert!(!approvals_enabled(Some(&json!({}))));
        assert!(!approvals_enabled(None));

        // The decoded-map entry point must answer identically.
        let enabled =
            std::collections::HashMap::from([(SLACK_APPROVAL_HINT.to_string(), json!(true))]);
        assert!(approvals_enabled_in(Some(&enabled)));
        assert!(!approvals_enabled_in(Some(
            &std::collections::HashMap::new()
        )));
        assert!(!approvals_enabled_in(None));
    }

    #[test]
    fn the_fallback_text_carries_the_ask() {
        let with_question = ApprovalRequest {
            action: "delete the staging database".to_string(),
            question: Some("Go ahead?".to_string()),
        };
        let text = approval_fallback_text(&with_question);
        assert!(text.contains("delete the staging database"));
        assert!(text.contains("Go ahead?"));

        let without = ApprovalRequest {
            action: "delete the staging database".to_string(),
            question: None,
        };
        assert!(approval_fallback_text(&without).contains("delete the staging database"));
    }
}
