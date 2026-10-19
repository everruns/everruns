//! One implementation of session commands, executed at the authority transaction.
//!
//! A host captures a bounded view, calls [`prepare`], and commits its effects,
//! receipt and activation obligation together. The view must remain locked (or
//! revision-checked) through commit. A remote client must never authorize a plan
//! by preparing it outside the authority and posting unchecked effects.
mod approval;
mod approval_outcome;
pub use approval_outcome::ApprovalOutcome;
mod causal;
mod configuration;
mod resolution;
#[cfg(test)]
mod tests;
pub use approval::ApprovalCommit;
use approval::{prepare_approval_submission, prepare_approvals};
pub use configuration::*;
use everruns_contracts::execution_api::ToolApprovalDecision;
pub use resolution::*;

use crate::events::{
    EventContext, EventRequest, InputMessageData, OutputMessageCompletedData, ToolCompletedData,
};
use crate::message::{RuntimeMessage, RuntimeMessageRole};
use everruns_contracts::{
    error::{AgentLoopError, Result},
    rt::UtcTimestamp,
    tool_types::ToolCall,
    typed_id::{EventId, MessageId, SessionId, TurnId},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

/// A semantic operation; platform adapters do not reimplement these rules.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionCommand {
    Create {
        binding: Value,
    },
    /// Retry restored code without migrating the immutable binding or resetting waits.
    RetryExecution {
        id: String,
        binding: Value,
    },
    Admit {
        message: Box<RuntimeMessage>,
        #[serde(default)]
        cancelled: bool,
    },
    Configure {
        id: String,
        patch: SessionConfiguration,
    },
    Seed {
        id: String,
        messages: Vec<RuntimeMessage>,
    },
    Approve {
        id: String,
        approvals: Vec<ApprovalCommit>,
    },
    SubmitApprovals {
        id: String,
        turn: TurnId,
        decisions: Vec<everruns_contracts::execution_api::ToolApprovalAnswer>,
    },
    Resolve {
        id: String,
        turn: TurnId,
        results: Vec<ToolCompletedData>,
    },
    ResolveWaits {
        id: String,
        turn: TurnId,
        approvals: Vec<ApprovalCommit>,
        results: Vec<ToolCompletedData>,
    },
    Cancel {
        id: String,
        input: MessageId,
    },
}

/// A retained command's immutable request fingerprint and reply.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandReceipt {
    pub fingerprint: Value,
    pub value: Value,
}

/// Bounded authority facts captured in the same transaction as the command.
#[derive(Clone, Serialize, Deserialize)]
pub struct CommandView {
    pub session: SessionId,
    pub now_ms: i64,
    #[serde(default)]
    pub binding: Option<Value>,
    /// Fail closed for adapters that have not captured the lease clock.
    #[serde(default = "active_by_default")]
    pub execution_active: bool,
    #[serde(default)]
    pub suspended: bool,
    #[serde(default)]
    pub suspension_reason: Option<String>,
    #[serde(default)]
    pub head: i32,
    #[serde(default)]
    pub open: Option<EventContext>,
    #[serde(default)]
    pub pending: Vec<ToolCall>,
    #[serde(default)]
    pub configuration: SessionConfiguration,
    #[serde(default)]
    pub previous: Option<CommandReceipt>,
    #[serde(default)]
    pub command_count: usize,
    #[serde(default)]
    pub queued: usize,
    #[serde(default)]
    pub has_history: bool,
    /// Facts for the admission named by a cancellation, never the latest turn.
    #[serde(default)]
    pub admitted: bool,
    #[serde(default)]
    pub assignment: Option<TurnId>,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default)]
    pub rejected: bool,
}

impl CommandView {
    /// Start with no authority facts. Storage adapters fill the fields read by
    /// their command while holding the session's transaction boundary.
    pub fn new(session: SessionId, now_ms: i64) -> Self {
        Self {
            session,
            now_ms,
            binding: None,
            execution_active: true,
            suspended: false,
            suspension_reason: None,
            head: 0,
            open: None,
            pending: Vec::new(),
            configuration: SessionConfiguration::default(),
            previous: None,
            command_count: 0,
            queued: 0,
            has_history: false,
            admitted: false,
            assignment: None,
            terminal: false,
            rejected: false,
        }
    }
}

fn active_by_default() -> bool {
    true
}

/// A transactional object write; `kind` is an authority storage namespace.
#[derive(Serialize)]
pub struct ObjectWrite {
    pub kind: &'static str,
    pub key: String,
    pub value: Value,
}

/// Small control updates derived by Rust, applied with the journal writes.
#[derive(Default, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlUpdate {
    #[default]
    None,
    Bind {
        binding: Value,
    },
    Admit {
        message: MessageId,
        cancelled: bool,
    },
    Resume,
    RetryExecution,
    Cancel {
        input: MessageId,
        interrupt: bool,
    },
}

/// A complete atomic commit, or a replay with no effects.
#[derive(Serialize)]
pub struct CommandPlan {
    pub key: Option<String>,
    pub fingerprint: Value,
    pub reply: Value,
    pub events: Vec<EventRequest>,
    pub objects: Vec<ObjectWrite>,
    pub control: ControlUpdate,
    pub replay: bool,
}

impl SessionCommand {
    /// Lookup before validating deadlines; an accepted retry returns its original reply.
    pub fn key(&self) -> Option<String> {
        Some(match self {
            Self::Create { .. } => return None,
            Self::RetryExecution { id, .. } => format!("execution_retry:{id}"),
            Self::Admit { message, .. } => message.id.to_string(),
            Self::Configure { id, .. } => format!("configuration:{id}"),
            Self::Seed { id, .. } => format!("seed:{id}"),
            Self::Approve { id, .. } => format!("approval:{id}"),
            Self::SubmitApprovals { id, .. } => format!("approval_submission:{id}"),
            Self::Resolve { id, .. } => format!("resolution:{id}"),
            Self::ResolveWaits { id, .. } => format!("wait_resolution:{id}"),
            Self::Cancel { id, .. } => format!("cancel:{id}"),
        })
    }
    pub fn fingerprint(&self) -> Value {
        match self {
            Self::Create { binding } => binding.clone(),
            Self::RetryExecution { binding, .. } => binding.clone(),
            Self::Admit { message, cancelled } => {
                let mut value = json!(message);
                if let Some(object) = value.as_object_mut() {
                    object.remove("created_at");
                    object.remove("updated_at");
                }
                causal::normalize_fingerprint(&mut value);
                if *cancelled {
                    json!({"message":value,"cancelled":true})
                } else {
                    value
                }
            }
            Self::Configure { patch, .. } => json!(patch),
            Self::Seed { messages, .. } => json!(messages),
            Self::Approve { approvals, .. } => json!({"approvals":approvals}),
            Self::SubmitApprovals {
                turn, decisions, ..
            } => json!({"turn_id":turn,"decisions":decisions}),
            Self::Resolve { turn, results, .. } => json!({"turn_id":turn,"results":results}),
            Self::ResolveWaits {
                turn,
                approvals,
                results,
                ..
            } => json!({"turn_id":turn,"approvals":approvals,"results":results}),
            Self::Cancel { input, .. } => json!({"input":input}),
        }
    }
}

/// Command failures preserve conflict and capacity semantics across transports.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    Invalid(#[from] AgentLoopError),
    #[error("{0}")]
    Conflict(&'static str),
    #[error("{0}")]
    Capacity(&'static str),
    #[error("{0}")]
    TooLarge(&'static str),
}
impl From<CommandError> for AgentLoopError {
    fn from(error: CommandError) -> Self {
        match error {
            CommandError::Invalid(error) => error,
            error => AgentLoopError::config(error.to_string()),
        }
    }
}
fn invalid(message: impl Into<String>) -> CommandError {
    AgentLoopError::config(message).into()
}

/// Validate and derive all effects before the host writes any member of a batch.
pub fn prepare(
    command: SessionCommand,
    view: CommandView,
) -> std::result::Result<CommandPlan, CommandError> {
    let mut plan = CommandPlan {
        key: command.key(),
        fingerprint: command.fingerprint(),
        reply: json!({}),
        events: Vec::new(),
        objects: Vec::new(),
        control: ControlUpdate::None,
        replay: false,
    };
    let identity = match &command {
        SessionCommand::Configure { id, .. }
        | SessionCommand::RetryExecution { id, .. }
        | SessionCommand::Seed { id, .. }
        | SessionCommand::Approve { id, .. }
        | SessionCommand::SubmitApprovals { id, .. }
        | SessionCommand::Resolve { id, .. }
        | SessionCommand::ResolveWaits { id, .. }
        | SessionCommand::Cancel { id, .. } => Some(id),
        _ => None,
    };
    if identity.is_some_and(|id| id.is_empty())
        || plan.key.as_ref().is_some_and(|key| {
            key.len() > 128
                || !key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
        })
    {
        return Err(invalid("invalid command identity"));
    }
    if !matches!(command, SessionCommand::Create { .. }) && view.binding.is_none() {
        return Err(AgentLoopError::SessionNotFound(view.session).into());
    }
    if let Some(previous) = view.previous {
        let mut fingerprint = previous.fingerprint;
        if matches!(command, SessionCommand::Admit { .. }) {
            // Normalize old retained receipts too; journals predating this
            // feature may already contain arbitrary metadata at this key.
            causal::normalize_fingerprint(&mut fingerprint);
        }
        if fingerprint != plan.fingerprint {
            return Err(CommandError::Conflict(
                "command identity reused with different input",
            ));
        }
        plan.reply = previous.value;
        plan.replay = true;
        return Ok(plan);
    }
    if plan.key.is_some() && view.command_count >= 100_000 {
        return Err(CommandError::Capacity("session command capacity reached"));
    }
    let now = UtcTimestamp::from_timestamp_millis(view.now_ms)
        .ok_or_else(|| invalid("invalid authority clock"))?;
    match command {
        SessionCommand::RetryExecution { binding, .. } => {
            // THREAT[TM-DURABLE-012]: this is a retry of restored trusted code,
            // never a build migration or a way around uncertain external effects.
            if view.binding.as_ref() != Some(&binding) {
                return Err(CommandError::Conflict(
                    "session binding changed before retry",
                ));
            }
            if view.execution_active {
                return Err(CommandError::Conflict("execution epoch is still active"));
            }
            if view.suspended {
                if !matches!(
                    view.suspension_reason.as_deref(),
                    Some(
                        "retained_build_unavailable"
                            | "retained_configuration_unavailable"
                            | "unsupported_runtime_version"
                            | "incompatible_executor_build"
                    )
                ) {
                    return Err(CommandError::Conflict(
                        "suspension requires effect reconciliation",
                    ));
                }
                plan.reply = json!({"retried_suspension":view.suspension_reason});
                plan.control = ControlUpdate::RetryExecution;
            }
        }
        SessionCommand::Create { binding } => {
            for field in ["agent", "build_id"] {
                let value = binding[field]
                    .as_str()
                    .ok_or_else(|| invalid("invalid agent build binding"))?;
                if value.is_empty()
                    || value.len() > 128
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
                {
                    return Err(invalid("invalid agent build binding"));
                }
            }
            if view.binding.as_ref().is_some_and(|old| old != &binding) {
                return Err(CommandError::Conflict("session binding conflict"));
            }
            plan.control = ControlUpdate::Bind { binding };
        }
        SessionCommand::Admit {
            mut message,
            cancelled,
        } => {
            crate::causal::sanitize_input_metadata(&mut message.metadata);
            if message.role != RuntimeMessageRole::User {
                return Err(invalid("admission requires user input"));
            }
            if view.queued >= 256 {
                return Err(CommandError::Capacity("session inbox is full"));
            }
            let sequence = view
                .head
                .checked_add(1)
                .ok_or_else(|| CommandError::Capacity("session sequence capacity reached"))?;
            plan.reply = json!({"message_id":message.id,"sequence":sequence});
            plan.control = ControlUpdate::Admit {
                message: message.id,
                cancelled,
            };
            plan.events.push(EventRequest::new(
                view.session,
                EventContext::empty(),
                InputMessageData::new(*message),
            ));
        }
        SessionCommand::Configure { patch, .. } => {
            patch.validate()?;
            let mut configuration = view.configuration;
            configuration.merge(patch);
            plan.objects.push(ObjectWrite {
                kind: "framework",
                key: "configuration".into(),
                value: json!(configuration),
            });
        }
        SessionCommand::Seed { messages, .. } => {
            validate_history_seed(&messages)?;
            plan.reply = json!(!view.has_history);
            if !view.has_history {
                for message in messages {
                    plan.events
                        .push(if message.role == RuntimeMessageRole::Agent {
                            EventRequest::new(
                                view.session,
                                EventContext::empty(),
                                OutputMessageCompletedData::new(message),
                            )
                        } else {
                            EventRequest::new(
                                view.session,
                                EventContext::empty(),
                                InputMessageData::new(message),
                            )
                        });
                }
            }
        }
        SessionCommand::Cancel { input, .. } => {
            if !view.admitted {
                return Err(invalid("admission not found"));
            }
            let interrupt = view.assignment.is_some_and(|turn| {
                view.open
                    .as_ref()
                    .is_none_or(|open| open.turn_id == Some(turn))
            }) && !view.terminal
                && !view.rejected;
            plan.control = ControlUpdate::Cancel { input, interrupt };
        }
        command => {
            if view.pending.len() > 256 {
                return Err(invalid("pending call capacity reached"));
            }
            let context = view
                .open
                .ok_or_else(|| invalid("session has no open turn"))?;
            let turn = context
                .turn_id
                .ok_or_else(|| invalid("open turn lacks identity"))?;
            if context.input_message_id.is_none() {
                return Err(invalid("open turn lacks input identity"));
            }
            let (batch, mut results) = match command {
                SessionCommand::Approve { approvals, .. } => (
                    prepare_approvals(&approvals, &view.pending, turn, now)?,
                    Vec::new(),
                ),
                SessionCommand::SubmitApprovals {
                    turn: target,
                    decisions,
                    ..
                } => {
                    if target != turn {
                        return Err(invalid("approval submission targets another turn"));
                    }
                    let (approvals, response) =
                        prepare_approval_submission(&decisions, &view.pending, turn, now)?;
                    plan.reply = json!(response);
                    (approvals, Vec::new())
                }
                SessionCommand::Resolve {
                    turn: target,
                    results,
                    ..
                } => {
                    if target != turn {
                        return Err(invalid("tool resolution targets another turn"));
                    }
                    (Vec::new(), results)
                }
                SessionCommand::ResolveWaits {
                    turn: target,
                    approvals,
                    results,
                    ..
                } => {
                    if target != turn {
                        return Err(invalid("wait resolution targets another turn"));
                    }
                    (
                        if approvals.is_empty() {
                            Vec::new()
                        } else {
                            prepare_approvals(&approvals, &view.pending, turn, now)?
                        },
                        results,
                    )
                }
                _ => return Err(invalid("invalid wait resolution command")),
            };
            if batch.len() + results.len() > 256 || batch.is_empty() && results.is_empty() {
                return Err(invalid("invalid wait resolution batch"));
            }
            // Some client protocols identify results by call id only. Resolve
            // the name from canonical state, never from client declarations.
            for result in &mut results {
                if result.tool_name.is_empty() {
                    result.tool_name = view
                        .pending
                        .iter()
                        .find(|call| call.id == result.tool_call_id)
                        .ok_or_else(|| invalid("tool result does not match a pending call"))?
                        .name
                        .clone();
                }
            }
            if !results.is_empty() {
                validate_tool_resolutions(&results, &view.pending, now)?;
            }
            let mut resolved = HashSet::new();
            for prepared in batch {
                resolved.insert(prepared.request_id);
                if let Some((key, record)) = prepared.record {
                    plan.objects.push(ObjectWrite {
                        kind: "value",
                        key,
                        value: json!(record),
                    });
                }
                plan.events.push(EventRequest::new(
                    view.session,
                    context.clone(),
                    prepared.result,
                ));
                plan.events.push(EventRequest::new(
                    view.session,
                    context.clone(),
                    InputMessageData::new(prepared.spoken),
                ));
            }
            for result in results {
                resolved.insert(result.tool_call_id.clone());
                plan.events
                    .push(EventRequest::new(view.session, context.clone(), result));
            }
            if view.pending.iter().all(|call| resolved.contains(&call.id)) {
                plan.control = ControlUpdate::Resume;
            }
        }
    }
    // Event timestamps, including synthetic answers, belong to the authority clock.
    for event in &mut plan.events {
        event.ts = now;
    }
    let count = i32::try_from(plan.events.len())
        .map_err(|_| CommandError::Capacity("session sequence capacity reached"))?;
    view.head
        .checked_add(count)
        .ok_or_else(|| CommandError::Capacity("session sequence capacity reached"))?;
    for event in &plan.events {
        if serde_json::to_vec(
            &event
                .clone()
                .into_event(EventId::from_uuid(uuid::Uuid::nil()), i32::MAX),
        )
        .map_err(|e| invalid(e.to_string()))?
        .len()
            > 2 * 1024 * 1024
        {
            return Err(CommandError::TooLarge("session event exceeds size limit"));
        }
    }
    for object in &plan.objects {
        if object.key.len() > 512
            || serde_json::to_vec(&object.value)
                .map_err(|e| invalid(e.to_string()))?
                .len()
                > 2 * 1024 * 1024
        {
            return Err(CommandError::TooLarge("session value exceeds size limit"));
        }
    }
    if serde_json::to_vec(&plan)
        .map_err(|e| invalid(e.to_string()))?
        .len()
        > 8 * 1024 * 1024
    {
        return Err(CommandError::TooLarge("session command exceeds size limit"));
    }
    Ok(plan)
}
