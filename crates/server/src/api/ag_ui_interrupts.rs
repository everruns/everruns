// AG-UI 1.0 interrupts and resume over the AG-UI endpoint.
//
// Spec: knowledge/integrations/ag-ui.md. A turn that parks on something only
// the outside world can give it ends its AG-UI run with the interrupt outcome,
// and the run that continues carries the answers in `RunAgentInput.resume`.
// Two parks become interrupts here:
//
// - `ask_user`: the interrupt id is the call id and `responseSchema` is the
//   answer object the shared question-resolution operation takes (EVE-1054),
//   the same schema the A2A channel advertises. A `secret` question cannot be
//   answered over AG-UI, because the value would travel through the agent's
//   own transcript; it can only be abandoned (TM-AGENT-016).
// - a hard tool-approval request (EVE-1140): answerable only on endpoints that
//   turn `tool_approval_interrupts` on. Elsewhere the interrupt says an
//   operator decides, and a client answer is refused, so an anonymous visitor
//   cannot approve a held-back action (TM-TOOL-052).
//
// Answers land on the same resolvers the session API uses, so validation,
// attribution and the single-claim resume cannot drift. Everything that says
// what was asked is read back from the parked `tool.call_requested`, never from
// the resume entry. Interrupt ids are looked up only on the thread's own
// session, which is what binds a resume to the thread that was interrupted.
//
// Producer rules from the 1.0 interrupt pattern: an entry naming an interrupt
// we did not raise is ignored with a warning; an interrupt with no entry is
// never treated as abandoned, so the run re-interrupts and resolves nothing.

use std::collections::HashMap;
use std::sync::Arc;

use crate::records::{AgUiChannelConfig, SessionStatus};
use everruns_ag_ui::{Interrupt, ResumeEntry, ResumeStatus};
use everruns_builtins::ask_user::{AskUserAnswer, AskUserQuestionKind, AskUserStatus};
use everruns_contracts::tool_types::ToolApprovalRequired;
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::ToolCallRequestedData;
use serde_json::{Value, json};

use crate::api::channel_a2a::ask_user::{ask_user_answer_schema, pending_ask_user_from_request};
use crate::api::question_answers::{
    PendingQuestions, QUESTION_LOOKBACK_EVENTS, QuestionAnswersRequest, QuestionResolver,
    ResolveError, SubmittedStatus,
};
use crate::api::tool_approvals::{
    ApprovalOutcome, ApprovalResolveError, ApprovalServices, PendingApproval, ToolApprovalAnswer,
    ToolApprovalDecision, resolve_tool_approvals, validate_decisions,
};
use crate::storage::StorageBackend;

/// `Interrupt.reason` for an `ask_user` question set.
pub(crate) const ASK_USER_REASON: &str = "everruns.ask_user";
/// `Interrupt.reason` for a question set that wants a credential.
pub(crate) const SECRET_REASON: &str = "everruns.secret_required";
/// `Interrupt.reason` for a tool approval the client may answer.
pub(crate) const TOOL_APPROVAL_REASON: &str = "tool_approval";
/// `Interrupt.reason` for a tool approval only an operator may answer.
pub(crate) const OPERATOR_APPROVAL_REASON: &str = "everruns.operator_approval";

/// What a parked call batch is waiting on, typed.
pub(crate) struct ParkedCalls {
    questions: Option<PendingQuestions>,
    approvals: Vec<PendingApproval>,
}

impl ParkedCalls {
    pub(crate) fn from_request(requested: &ToolCallRequestedData) -> Self {
        let approvals = requested
            .tool_calls
            .iter()
            .filter_map(|call| {
                let request =
                    ToolApprovalRequired::from_request_call(&call.id, &call.name, &call.arguments)?;
                let expires_at = chrono::DateTime::parse_from_rfc3339(&request.expires_at)
                    .ok()
                    .map(|value| value.with_timezone(&chrono::Utc));
                Some(PendingApproval {
                    request_call_id: call.id.clone(),
                    request,
                    expires_at,
                })
            })
            .collect();
        Self {
            questions: pending_ask_user_from_request(requested),
            approvals,
        }
    }

    /// Whether the call is a question or an approval, answered by a resume
    /// entry rather than by a frontend tool result.
    pub(crate) fn claims(&self, call_id: &str) -> bool {
        self.questions
            .as_ref()
            .is_some_and(|pending| pending.tool_call_id == call_id)
            || self
                .approvals
                .iter()
                .any(|approval| approval.request_call_id == call_id)
    }

    /// The interrupts this batch ends the run with. Empty when it waits on
    /// nothing AG-UI can name (a client-side tool, for example).
    pub(crate) fn interrupts(&self, config: &AgUiChannelConfig) -> Vec<Interrupt> {
        let mut interrupts = Vec::new();
        if let Some(pending) = &self.questions {
            interrupts.push(question_interrupt(pending));
        }
        for approval in &self.approvals {
            interrupts.push(approval_interrupt(approval, config));
        }
        interrupts
    }
}

fn question_interrupt(pending: &PendingQuestions) -> Interrupt {
    let secret = pending
        .questions
        .iter()
        .any(|question| question.kind == AskUserQuestionKind::Secret);
    let message = pending
        .questions
        .iter()
        .map(|question| question.question.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut interrupt = Interrupt {
        message: Some(message),
        expires_at: pending.expires_at.map(|at| at.to_rfc3339()),
        ..Interrupt::new(
            pending.tool_call_id.clone(),
            if secret {
                SECRET_REASON
            } else {
                ASK_USER_REASON
            },
        )
    };
    if !secret {
        interrupt.response_schema = as_object(ask_user_answer_schema(pending));
        interrupt.metadata = Some(everruns_metadata(json!({
            "questions": pending.questions,
        })));
    }
    interrupt
}

fn approval_interrupt(approval: &PendingApproval, config: &AgUiChannelConfig) -> Interrupt {
    let request = &approval.request;
    let tool = request.display_name.as_deref().unwrap_or(&request.tool);
    if !config.tool_approval_interrupts {
        return Interrupt {
            message: Some(format!(
                "Waiting for an operator to approve a call to {tool}."
            )),
            expires_at: Some(request.expires_at.clone()),
            ..Interrupt::new(approval.request_call_id.clone(), OPERATOR_APPROVAL_REASON)
        };
    }
    Interrupt {
        message: Some(format!("Allow the agent to run {tool}?")),
        tool_call_id: Some(request.tool_call_id.clone()),
        expires_at: Some(request.expires_at.clone()),
        response_schema: as_object(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["decision"],
            "properties": {
                "decision": {
                    "enum": ["allow", "allow_always", "reject", "reject_always"],
                    "description": "`*_always` applies to every later call of this tool in the thread.",
                },
            },
        })),
        metadata: Some(everruns_metadata(json!({
            "tool": request.tool,
            "arguments": request.arguments,
            "arguments_truncated": request.arguments_truncated,
            "risk": request.risk,
        }))),
        ..Interrupt::new(approval.request_call_id.clone(), TOOL_APPROVAL_REASON)
    }
}

fn as_object(value: Value) -> Option<serde_json::Map<String, Value>> {
    match value {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Our keys go under `everruns`; `ag-ui` is reserved for the protocol.
fn everruns_metadata(value: Value) -> everruns_ag_ui::Metadata {
    let mut metadata = everruns_ag_ui::Metadata::new();
    metadata.insert("everruns".to_string(), value);
    metadata
}

/// What a resuming run continues from.
pub(crate) enum ResumeOutcome {
    /// The session was not parked: the entries answer nothing.
    NothingParked,
    /// Some interrupt or frontend tool call has no answer: nothing was
    /// resolved, and the run ends asking for all of them again.
    StillOpen {
        tool_calls: Vec<everruns_ag_ui::ToolCall>,
        interrupts: Vec<Interrupt>,
    },
    /// Every interrupt was resolved and the turn resumed. Its events carry
    /// this input message id, when the parked event recorded one.
    Resumed { input_message_id: Option<String> },
}

/// Why a resume input is refused before its run starts.
#[derive(Debug)]
pub(crate) enum ResumeError {
    /// The entries cannot be applied as sent.
    Invalid(String),
    /// Someone else already answered, or the deadline passed.
    Conflict(String),
    Internal(anyhow::Error),
}

pub(crate) struct ResumeServices<'a> {
    pub(crate) db: &'a Arc<StorageBackend>,
    pub(crate) session_service: &'a crate::domains::sessions::SessionService,
    pub(crate) event_service: &'a crate::services::EventService,
    pub(crate) runner: Arc<dyn everruns_worker::AgentRunner>,
}

/// Apply a run's resume entries to the thread's session.
pub(crate) async fn resume(
    services: &ResumeServices<'_>,
    org_id: i64,
    session: &crate::records::Session,
    config: &AgUiChannelConfig,
    entries: &[ResumeEntry],
) -> Result<ResumeOutcome, ResumeError> {
    if session.status != SessionStatus::WaitingForToolResults {
        warn_unrecognised(session.id, entries.iter());
        return Ok(ResumeOutcome::NothingParked);
    }
    // THREAT[TM-TENANT-016]: `session` is the one the request's own routing
    // tags resolved, so an entry can only answer this thread's interrupts.
    let rows = services
        .db
        .list_events(
            session.id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(QUESTION_LOOKBACK_EVENTS),
        )
        .await
        .map_err(ResumeError::Internal)?;
    let Some(row) = rows.last() else {
        warn_unrecognised(session.id, entries.iter());
        return Ok(ResumeOutcome::NothingParked);
    };
    let requested: ToolCallRequestedData = serde_json::from_value(row.data.clone())
        .map_err(|error| ResumeError::Internal(error.into()))?;
    let parked = ParkedCalls::from_request(&requested);
    let interrupts = parked.interrupts(config);
    if interrupts.is_empty() {
        warn_unrecognised(session.id, entries.iter());
        return Ok(ResumeOutcome::NothingParked);
    }

    let by_id: HashMap<&str, &ResumeEntry> = entries
        .iter()
        .map(|entry| (entry.interrupt_id.as_str(), entry))
        .collect();
    warn_unrecognised(
        session.id,
        entries.iter().filter(|entry| {
            !interrupts
                .iter()
                .any(|interrupt| interrupt.id == entry.interrupt_id)
        }),
    );
    if interrupts
        .iter()
        .any(|interrupt| !by_id.contains_key(interrupt.id.as_str()))
    {
        return Ok(ResumeOutcome::StillOpen {
            tool_calls: Vec::new(),
            interrupts,
        });
    }

    // Validate every entry before resolving any, so a bad approval cannot
    // leave a question answered and the turn half-resumed.
    let question = match &parked.questions {
        Some(pending) => Some((
            pending,
            question_answer(pending, by_id[pending.tool_call_id.as_str()])?,
        )),
        None => None,
    };
    let approvals = if parked.approvals.is_empty() {
        None
    } else {
        let answers = parked
            .approvals
            .iter()
            .map(|approval| {
                approval_answer(approval, by_id[approval.request_call_id.as_str()], config)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let outcomes = validate_decisions(&parked.approvals, &answers, chrono::Utc::now())
            .map_err(approval_error)?;
        Some(outcomes)
    };

    let caller = everruns_core::Caller::internal(org_id);
    if let Some((pending, (status, answers))) = question {
        let resolver = QuestionResolver {
            db: services.db,
            session_service: services.session_service,
            event_service: services.event_service,
            runner: services.runner.clone(),
        };
        crate::api::question_answers::resolve_question_answers(
            &resolver,
            &caller,
            session.id,
            Some(&pending.tool_call_id),
            status,
            &answers,
        )
        .await
        .map_err(question_error)?;
    }
    if let Some(outcomes) = approvals {
        resolve_tool_approvals(
            &ApprovalServices {
                db: services.db,
                event_service: services.event_service,
                runner: &services.runner,
            },
            org_id,
            session.id,
            &parked.approvals,
            &outcomes,
            ApprovalOutcome::NotApproved,
            "ag_ui_resume",
        )
        .await
        .map_err(approval_error)?;
    }

    Ok(ResumeOutcome::Resumed {
        input_message_id: row
            .context
            .get("input_message_id")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn warn_unrecognised<'a>(session_id: SessionId, entries: impl Iterator<Item = &'a ResumeEntry>) {
    for entry in entries {
        tracing::warn!(
            session_id = %session_id,
            interrupt_id = %entry.interrupt_id,
            "AG-UI resume entry names no open interrupt; ignoring it"
        );
    }
}

/// An `ask_user` entry as the resolver's status and answers.
fn question_answer(
    pending: &PendingQuestions,
    entry: &ResumeEntry,
) -> Result<(AskUserStatus, Vec<AskUserAnswer>), ResumeError> {
    if entry.status == ResumeStatus::Cancelled {
        return Ok((AskUserStatus::Declined, Vec::new()));
    }
    if pending
        .questions
        .iter()
        .any(|question| question.kind == AskUserQuestionKind::Secret)
    {
        return Err(ResumeError::Invalid(
            "a credential cannot be sent over AG-UI; abandon the interrupt instead".to_string(),
        ));
    }
    let payload = entry
        .payload
        .clone()
        .ok_or_else(|| ResumeError::Invalid("a resolved entry needs a payload".to_string()))?;
    let request: QuestionAnswersRequest = serde_json::from_value(payload)
        .map_err(|error| ResumeError::Invalid(format!("invalid ask_user answer: {error}")))?;
    if request
        .tool_call_id
        .as_deref()
        .is_some_and(|id| id != pending.tool_call_id)
    {
        return Err(ResumeError::Invalid(
            "the answer names a different question set".to_string(),
        ));
    }
    let answers: Vec<AskUserAnswer> = request.answers.into_iter().map(Into::into).collect();
    if answers.iter().any(|answer| answer.secret_ref.is_some()) {
        return Err(ResumeError::Invalid(
            "secret_ref is not accepted over AG-UI".to_string(),
        ));
    }
    let status = match request.status {
        SubmittedStatus::Answered => AskUserStatus::Answered,
        SubmittedStatus::Declined => AskUserStatus::Declined,
    };
    Ok((status, answers))
}

/// A tool-approval entry as a decision.
fn approval_answer(
    approval: &PendingApproval,
    entry: &ResumeEntry,
    config: &AgUiChannelConfig,
) -> Result<ToolApprovalAnswer, ResumeError> {
    let decision = if entry.status == ResumeStatus::Cancelled {
        // Abandoning never runs the held-back call.
        ToolApprovalDecision::Reject
    } else if !config.tool_approval_interrupts {
        // THREAT[TM-TOOL-052]: on an endpoint that did not opt in, the client
        // is not who approves; only an operator can let the call run.
        return Err(ResumeError::Invalid(
            "this endpoint does not accept tool approvals; an operator decides".to_string(),
        ));
    } else {
        let decision = entry
            .payload
            .as_ref()
            .and_then(|payload| payload.get("decision"))
            .cloned()
            .ok_or_else(|| ResumeError::Invalid("a tool approval needs a decision".to_string()))?;
        serde_json::from_value(decision)
            .map_err(|error| ResumeError::Invalid(format!("invalid decision: {error}")))?
    };
    Ok(ToolApprovalAnswer {
        tool_call_id: approval.request_call_id.clone(),
        decision,
    })
}

fn question_error(error: ResolveError) -> ResumeError {
    match error {
        ResolveError::Invalid(detail) => ResumeError::Invalid(detail),
        ResolveError::AlreadyResolved => {
            ResumeError::Conflict("the question set was already answered".to_string())
        }
        ResolveError::NotWaiting(status) => {
            ResumeError::Conflict(format!("the thread is not waiting (status: {status})"))
        }
        ResolveError::NoPendingQuestions | ResolveError::WrongPendingCall => {
            ResumeError::Conflict("the thread is not waiting on that question set".to_string())
        }
        ResolveError::Internal(detail) => ResumeError::Internal(anyhow::anyhow!(detail)),
    }
}

fn approval_error(error: ApprovalResolveError) -> ResumeError {
    match error {
        ApprovalResolveError::Invalid(detail) => ResumeError::Invalid(detail),
        ApprovalResolveError::NotFound(detail) => ResumeError::Conflict(detail),
        ApprovalResolveError::AlreadyResolved => {
            ResumeError::Conflict("the approval requests were already answered".to_string())
        }
        ApprovalResolveError::Expired => {
            ResumeError::Conflict("the approval request expired and counts as rejected".to_string())
        }
        ApprovalResolveError::NotWaiting(status) => {
            ResumeError::Conflict(format!("the thread is not waiting (status: {status})"))
        }
        ApprovalResolveError::Internal(detail) => ResumeError::Internal(anyhow::anyhow!(detail)),
    }
}
