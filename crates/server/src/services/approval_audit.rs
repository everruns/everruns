// Approval audit listener — turns soft-approval tool calls into org audit rows.
//
// The `soft_approval` capability records consent as a tool call, which lands in
// the session's event log. That is the right place for the conversation's own
// record, but it is the wrong place to *answer to*: session events are scoped
// to one session, they are pruned with it, and nothing in them names the
// person who said yes.
//
// Identity is deliberately not supplied by the tool. A model-written
// `approved_by` is an assertion by the thing being governed, which is not
// evidence. So the tools record only the turn and the input message the
// consent was spoken in, and this listener resolves the approver from the
// server's own authenticated record: the `input.message` event whose metadata
// the API wrote from the authenticated caller (see
// `execution_metadata::interactive_user_metadata`).
//
// The resolved actor is therefore the identity that sent the consenting
// message. For a scheduled or trigger-driven turn there is no human in the
// loop, and the initiator resolves to that non-human initiator instead. Such a
// row still records that an approval was claimed, and the absent human actor is
// itself the audit signal.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::{Event, EventData, EventListener, TOOL_COMPLETED};
use everruns_platform::{AgentAction, AuditEvent};
use everruns_provider::typed_id::SessionId;
use serde_json::Value;
use tracing::instrument;
use uuid::Uuid;

use crate::auth::audit;
use crate::storage::StorageBackend;

/// The soft-approval tools whose completions become audit rows.
const REQUEST_APPROVAL_TOOL: &str = "request_approval";
const RECORD_APPROVAL_TOOL: &str = "record_approval";

const INPUT_MESSAGE_EVENT: &str = "input.message";

/// Writes an org-level audit row for every approval asked for and granted.
pub struct ApprovalAuditListener {
    db: Arc<StorageBackend>,
}

impl ApprovalAuditListener {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    /// The identity that sent the last user message before `before_sequence`.
    ///
    /// That message is the consent: `record_approval` is called in the turn the
    /// affirmative reply started. Reading the initiator from the event the API
    /// itself wrote is what keeps the attribution out of the model's hands.
    async fn approver(
        &self,
        session_id: SessionId,
        before_sequence: i32,
    ) -> (Option<Uuid>, Option<String>) {
        let rows = match self
            .db
            .list_events(
                session_id,
                None,
                None,
                &[INPUT_MESSAGE_EVENT.to_string()],
                &[],
                Some(before_sequence),
                Some(1),
            )
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                // A row with no actor still beats no row: the approval happened
                // whether or not this lookup did.
                tracing::warn!(%session_id, %error, "could not resolve approval actor");
                return (None, None);
            }
        };

        let Some(metadata) = rows.first().and_then(|row| row.metadata.as_ref()) else {
            return (None, None);
        };
        (
            initiator_user_id(metadata),
            metadata
                .get("acting_principal_id")
                .and_then(Value::as_str)
                .map(str::to_string),
        )
    }
}

/// `{"initiator": {"type": "user", "user_id": "…"}}`, as written by
/// `execution_metadata::interactive_user_metadata`. A non-user initiator
/// (schedule, trigger) yields `None`, which is the honest answer.
fn initiator_user_id(metadata: &Value) -> Option<Uuid> {
    let initiator = metadata.get("initiator")?;
    if initiator.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }
    initiator
        .get("user_id")
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())
}

/// The first text part of a tool result, parsed back into JSON. Tool results
/// are carried as content parts, so the structured payload the tool returned
/// needs unwrapping before its fields are readable.
fn result_payload(parts: &[everruns_core::ContentPart]) -> Option<Value> {
    parts.iter().find_map(|part| match part {
        everruns_core::ContentPart::Text(part) => serde_json::from_str(&part.text).ok(),
        _ => None,
    })
}

/// The audit action a completed tool call maps to, if any.
///
/// `set_approval_mode` is deliberately absent: changing the level is a
/// configuration change, not consent for an action.
fn audited_action(tool_name: &str) -> Option<AgentAction> {
    match tool_name {
        REQUEST_APPROVAL_TOOL => Some(AgentAction::ApprovalRequested),
        RECORD_APPROVAL_TOOL => Some(AgentAction::ApprovalGranted),
        _ => None,
    }
}

/// Cap on each free-text detail copied into an audit row.
///
/// THREAT[TM-OBS-007] These strings are model-authored from the conversation
/// and land in `audit_logs`, which `AUDIT_LOG_VIEW` exposes to org admins who
/// may have no access to the session itself. The full text stays in the
/// session event log; the audit row carries a bounded excerpt plus the
/// correlation ids needed to go read the rest in context. Bounding also keeps
/// a model that emits a huge argument from writing an unbounded row
/// (TM-DOS-*).
const MAX_DETAIL_CHARS: usize = 512;

fn detail_str(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(bounded_detail)
}

/// Truncate on a character boundary, marking that it happened so a reader does
/// not mistake an excerpt for the whole value.
fn bounded_detail(value: &str) -> String {
    if value.chars().count() <= MAX_DETAIL_CHARS {
        return value.to_string();
    }
    let kept: String = value.chars().take(MAX_DETAIL_CHARS).collect();
    format!("{kept}… (truncated)")
}

#[async_trait]
impl EventListener for ApprovalAuditListener {
    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![TOOL_COMPLETED])
    }

    #[instrument(skip(self, event), fields(session_id = %event.session_id))]
    async fn on_event(&self, event: &Event) {
        let EventData::ToolCompleted(data) = &event.data else {
            return;
        };
        let Some(action) = audited_action(&data.tool_name) else {
            return;
        };
        // A failed call asked for nothing and granted nothing.
        if !data.success {
            return;
        }
        let Some(sequence) = event.sequence else {
            return;
        };
        let payload = data
            .result
            .as_deref()
            .and_then(result_payload)
            .unwrap_or(Value::Null);

        let Ok(Some(org_id)) = self.db.get_session_organization_id(event.session_id).await else {
            tracing::warn!(session_id = %event.session_id, "no org for approval audit row");
            return;
        };

        // The ask is attributed to whoever was in the conversation when it was
        // raised; the grant to whoever answered it.
        let (actor, principal_id) = self.approver(event.session_id, sequence).await;

        let mut audit_event = AuditEvent::agent(action, org_id, actor)
            .target("session", event.session_id.to_string())
            .detail("session_id", event.session_id.to_string())
            .detail("tool_name", data.tool_name.clone());
        if let Some(value) = detail_str(&payload, "action") {
            audit_event = audit_event.detail("approved_action", value);
        }
        if let Some(value) = detail_str(&payload, "detail") {
            audit_event = audit_event.detail("approved_detail", value);
        }
        if let Some(value) = detail_str(&payload, "question") {
            audit_event = audit_event.detail("question", value);
        }
        for key in ["approved_in_turn", "approved_in_message", "asked_in_turn"] {
            if let Some(value) = detail_str(&payload, key) {
                audit_event = audit_event.detail(key, value);
            }
        }
        if let Some(principal_id) = principal_id {
            audit_event = audit_event.detail("acting_principal_id", principal_id);
        }
        if actor.is_none() {
            // Explicit, because "no actor" here is a finding, not a gap in the
            // record: nobody human was identified as consenting.
            audit_event = audit_event.detail("actor_resolution", "unattributed");
        }

        audit::emit_event(self.db.clone(), audit_event.build());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn initiator_is_read_only_from_a_user_initiator() {
        let user_id = Uuid::new_v4();
        assert_eq!(
            initiator_user_id(&json!({
                "initiator": { "type": "user", "user_id": user_id },
                "acting_principal_id": "principal_abc",
            })),
            Some(user_id)
        );
        // A scheduled turn has an initiator, but not a human one.
        assert_eq!(
            initiator_user_id(&json!({
                "initiator": { "type": "schedule", "schedule_id": "sched_1" },
            })),
            None
        );
        assert_eq!(initiator_user_id(&json!({})), None);
        assert_eq!(
            initiator_user_id(&json!({"initiator": {"type": "user", "user_id": "not-a-uuid"}})),
            None
        );
    }

    /// Audit rows are read by org admins who may not have session access, so
    /// what the model wrote is excerpted rather than copied wholesale.
    #[test]
    fn free_text_details_are_bounded_on_a_character_boundary() {
        let short = "drop the staging database";
        assert_eq!(bounded_detail(short), short);

        // Exactly at the cap is not truncated.
        let at_cap = "a".repeat(MAX_DETAIL_CHARS);
        assert_eq!(bounded_detail(&at_cap), at_cap);

        let over = "a".repeat(MAX_DETAIL_CHARS + 1);
        let bounded = bounded_detail(&over);
        assert!(bounded.ends_with("… (truncated)"));
        assert_eq!(
            bounded.chars().count(),
            MAX_DETAIL_CHARS + "… (truncated)".chars().count()
        );

        // Multi-byte input must not panic or split a character.
        let wide = "界".repeat(MAX_DETAIL_CHARS + 10);
        let bounded = bounded_detail(&wide);
        assert!(bounded.starts_with(&"界".repeat(MAX_DETAIL_CHARS)));
        assert!(bounded.ends_with("… (truncated)"));

        // And it applies through the payload reader.
        let payload = json!({ "action": "x".repeat(MAX_DETAIL_CHARS + 50) });
        assert!(
            detail_str(&payload, "action")
                .expect("action")
                .ends_with("… (truncated)")
        );
    }

    #[test]
    fn the_grant_payload_is_read_back_out_of_the_content_part() {
        let parts = vec![everruns_core::ContentPart::text(
            json!({
                "recorded": true,
                "action": "drop the staging database",
                "approved_in_message": "message_abc",
            })
            .to_string(),
        )];
        let payload = result_payload(&parts).expect("payload");
        assert_eq!(
            detail_str(&payload, "action").as_deref(),
            Some("drop the staging database")
        );
        assert_eq!(
            detail_str(&payload, "approved_in_message").as_deref(),
            Some("message_abc")
        );
        // Absent and blank fields are both simply not recorded.
        assert_eq!(detail_str(&payload, "detail"), None);
        assert_eq!(detail_str(&json!({"action": "   "}), "action"), None);
    }

    #[test]
    fn only_the_two_approval_tools_are_audited() {
        assert_eq!(
            audited_action("request_approval"),
            Some(AgentAction::ApprovalRequested)
        );
        assert_eq!(
            audited_action("record_approval"),
            Some(AgentAction::ApprovalGranted)
        );
        // Changing the level is configuration, not consent.
        assert_eq!(audited_action("set_approval_mode"), None);
        assert_eq!(audited_action("write_todos"), None);
        assert_eq!(
            AgentAction::ApprovalGranted.as_str(),
            "agent.approval.granted"
        );
        assert_eq!(
            AgentAction::ApprovalRequested.as_str(),
            "agent.approval.requested"
        );
    }
}
