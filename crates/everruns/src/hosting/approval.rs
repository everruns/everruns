//! Shared semantic approval preparation. Authorities validate the retained
//! request and commit the decision, answer and wakeup in one transaction.
use super::*;
pub use everruns_contracts::execution_api::ToolApprovalDecision;
use everruns_contracts::{
    error::AgentLoopError,
    tool_approval_types::{
        StoredToolApproval, ToolApprovalRequired, always_decision_storage_key,
        one_off_decision_storage_key,
    },
};

/// A decision targeting the exact canonical request, never client-supplied
/// arguments or a free-form permission record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalCommit {
    /// Exact logical turn that issued this request. Call ids can repeat across turns.
    pub turn_id: everruns_contracts::typed_id::TurnId,
    /// Synthetic canonical request identity.
    pub request_id: String,
    /// The person's explicit decision.
    pub decision: ToolApprovalDecision,
}

/// Opaque command materialization for an authority plugin. Rust owns approval
/// policy; the authority compares the pending request and deadline, then
/// commits these writes and its wakeup obligation atomically.
#[derive(Serialize, Deserialize)]
pub struct PreparedApproval {
    /// Exact canonical request the transaction must still find pending.
    pub expected_request: crate::ToolCall,
    /// Authority-clock upper bound for committing the answer.
    pub not_after_ms: i64,
    /// Reserved session-control key.
    pub key: String,
    /// Serialized decision record from the shared policy implementation.
    pub record: String,
    /// Correlated synthetic tool answer.
    pub result: ToolCompletedData,
    /// The person's decision as an input to the same logical turn.
    pub spoken: RuntimeMessage,
}

impl ApprovalCommit {
    /// Materialize against an authority snapshot. The plugin must recheck
    /// `expected_request` and `not_after_ms` in its committing transaction.
    pub fn prepare_at(
        &self,
        pending: &[crate::ToolCall],
        turn: everruns_contracts::typed_id::TurnId,
        now: everruns_contracts::rt::UtcTimestamp,
    ) -> Result<PreparedApproval> {
        let call = pending
            .iter()
            .find(|call| call.id == self.request_id)
            .ok_or_else(|| AgentLoopError::config("approval request is no longer pending"))?;
        let request =
            ToolApprovalRequired::from_request_call(&call.id, &call.name, &call.arguments)
                .ok_or_else(|| AgentLoopError::config("invalid canonical approval request"))?;
        if request.is_expired(now) {
            return Err(AgentLoopError::config("approval request expired"));
        }
        let (key, mut record) = match self.decision {
            ToolApprovalDecision::Allow | ToolApprovalDecision::Reject => (
                one_off_decision_storage_key(&request.fingerprint),
                StoredToolApproval::one_off(
                    &request.tool,
                    &request.fingerprint,
                    self.decision.allows(),
                    now,
                ),
            ),
            ToolApprovalDecision::AllowAlways | ToolApprovalDecision::RejectAlways => (
                always_decision_storage_key(&request.tool),
                StoredToolApproval::always(&request.tool, self.decision.allows(), now),
            ),
        };
        if record.fingerprint.is_some() {
            record.turn_id = Some(turn);
        }
        let spoken = match self.decision {
            ToolApprovalDecision::Allow => format!(
                "I approved `{}` this once. Call it again now with exactly the same arguments.",
                request.tool
            ),
            ToolApprovalDecision::AllowAlways => format!(
                "I approved `{}` for the rest of this session. Call it again now with the same arguments.",
                request.tool
            ),
            ToolApprovalDecision::Reject => format!(
                "I rejected `{}`. Don't run it, and don't try to get the same result another way.",
                request.tool
            ),
            ToolApprovalDecision::RejectAlways => format!(
                "I rejected `{}` for the rest of this session. Don't call it again.",
                request.tool
            ),
        };
        Ok(PreparedApproval {
            expected_request: call.clone(),
            not_after_ms: request
                .expires_at
                .parse::<everruns_contracts::rt::UtcTimestamp>()
                .map_err(|_| AgentLoopError::config("invalid approval deadline"))?
                .timestamp_millis(),
            key,
            record: serde_json::to_string(&record)
                .map_err(|_| AgentLoopError::config("invalid approval record"))?,
            result: ToolCompletedData::success(
                call.id.clone(),
                call.name.clone(),
                vec![crate::ContentPart::tool_result_text(
                    &serde_json::json!({"approved":self.decision.allows(),"decision":self.decision}),
                )],
                None,
            ),
            spoken: RuntimeMessage::user(spoken),
        })
    }
}

/// Validate a complete approval batch before an authority applies any writes.
pub fn prepare_approvals(
    approvals: &[ApprovalCommit],
    pending: &[crate::ToolCall],
    turn: everruns_contracts::typed_id::TurnId,
    now: everruns_contracts::rt::UtcTimestamp,
) -> Result<Vec<PreparedApproval>> {
    if approvals.is_empty() || approvals.len() > 256 {
        return Err(AgentLoopError::config(
            "approval batch must contain between 1 and 256 decisions",
        ));
    }
    if approvals.iter().any(|approval| approval.turn_id != turn) {
        return Err(AgentLoopError::config("approval targets another turn"));
    }
    if approvals.iter().any(|approval| approval.turn_id != turn) {
        return Err(AgentLoopError::config("approval targets another turn"));
    }
    let mut seen = std::collections::HashSet::new();
    approvals
        .iter()
        .map(|approval| {
            if !seen.insert(&approval.request_id) {
                return Err(AgentLoopError::config("duplicate approval request"));
            }
            approval.prepare_at(pending, turn, now)
        })
        .collect()
}
