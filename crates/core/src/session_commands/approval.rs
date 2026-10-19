//! Shared semantic approval preparation. Authorities validate the retained
//! request and commit the decision, answer and wakeup in one transaction.
use super::*;
use everruns_contracts::execution_api::ToolApprovalDecision;
use everruns_contracts::execution_api::{
    SubmitToolApprovalsResponse, ToolApprovalAnswer, ToolApprovalResolution,
};
use everruns_contracts::{error::AgentLoopError, tool_approval_types::ToolApprovalRequired};

/// Expand an Execution API batch, rejecting omitted requests, before one commit.
pub(super) fn prepare_approval_submission(
    decisions: &[ToolApprovalAnswer],
    pending: &[ToolCall],
    turn: everruns_contracts::typed_id::TurnId,
    now: UtcTimestamp,
) -> Result<(Vec<PreparedApproval>, SubmitToolApprovalsResponse)> {
    let mut seen = std::collections::HashSet::new();
    if decisions.is_empty()
        || decisions.len() > 64
        || decisions
            .iter()
            .any(|answer| !seen.insert(&answer.tool_call_id))
    {
        return Err(AgentLoopError::config("invalid approval decisions"));
    }
    let requested: Vec<_> = pending
        .iter()
        .filter_map(|call| {
            ToolApprovalRequired::from_request_call(&call.id, &call.name, &call.arguments)
                .map(|request| (call, request))
        })
        .collect();
    if decisions.iter().any(|answer| {
        !requested
            .iter()
            .any(|(call, _)| call.id == answer.tool_call_id)
    }) {
        return Err(AgentLoopError::config(
            "approval request is no longer pending",
        ));
    }
    let mut approvals = Vec::new();
    let mut resolved = Vec::new();
    for (call, request) in requested {
        let explicit = decisions
            .iter()
            .find(|answer| answer.tool_call_id == call.id)
            .map(|answer| answer.decision);
        let outcome = explicit
            .map(ApprovalOutcome::Decided)
            .unwrap_or(ApprovalOutcome::NotApproved);
        approvals.push(prepare_approval(&call.id, pending, turn, outcome, now)?);
        resolved.push(ToolApprovalResolution {
            tool_call_id: call.id.clone(),
            tool: request.tool,
            outcome: outcome.label().into(),
        });
    }
    let status = if pending.len() == approvals.len() {
        "active"
    } else {
        "waitingfortoolresults"
    };
    Ok((
        approvals,
        SubmitToolApprovalsResponse {
            resolved,
            status: status.into(),
        },
    ))
}

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

/// Internal materialization; never accepted from a remote caller.
pub(super) struct PreparedApproval {
    pub request_id: String,
    pub record: Option<(String, String)>,
    pub result: ToolCompletedData,
    pub spoken: RuntimeMessage,
}

fn prepare_approval(
    request_id: &str,
    pending: &[ToolCall],
    turn: TurnId,
    outcome: ApprovalOutcome,
    now: UtcTimestamp,
) -> Result<PreparedApproval> {
    let call = pending
        .iter()
        .find(|call| call.id == request_id)
        .ok_or_else(|| AgentLoopError::config("approval request is no longer pending"))?;
    let request = ToolApprovalRequired::from_request_call(&call.id, &call.name, &call.arguments)
        .ok_or_else(|| AgentLoopError::config("invalid canonical approval request"))?;
    if request.is_expired(now) {
        return Err(AgentLoopError::config("approval request expired"));
    }
    let record = outcome
        .stored_record(&request, now)
        .map(|(key, mut record)| {
            if record.fingerprint.is_some() {
                record.turn_id = Some(turn);
            }
            serde_json::to_string(&record)
                .map(|value| (key, value))
                .map_err(|_| AgentLoopError::config("invalid approval record"))
        })
        .transpose()?;
    Ok(PreparedApproval {
        request_id: call.id.clone(),
        record,
        result: ToolCompletedData::success(
            call.id.clone(),
            call.name.clone(),
            vec![crate::ContentPart::tool_result_text(
                &json!({"approved":outcome.allows(),"decision":outcome.label()}),
            )],
            None,
        ),
        spoken: RuntimeMessage::user(outcome.spoken(&request.tool)),
    })
}

/// Validate a complete approval batch before an authority applies any writes.
pub(super) fn prepare_approvals(
    approvals: &[ApprovalCommit],
    pending: &[ToolCall],
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
    let mut seen = std::collections::HashSet::new();
    approvals
        .iter()
        .map(|approval| {
            if !seen.insert(&approval.request_id) {
                return Err(AgentLoopError::config("duplicate approval request"));
            }
            prepare_approval(
                &approval.request_id,
                pending,
                turn,
                ApprovalOutcome::Decided(approval.decision),
                now,
            )
        })
        .collect()
}
