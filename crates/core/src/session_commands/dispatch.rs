//! Authority-owned external dispatch claims. A claim is never reassigned: loss
//! after claim is an unknown outcome, not permission to repeat a mutation.
//! THREAT[TM-DURABLE-016]: exact pending-call identity, immutable owner/fence, no replay.
//! THREAT[TM-DURABLE-017]: terminal state, result and activation share one authority commit.
//! Owner UUIDs fence concurrency; the embedding host must authenticate session routing.
use super::*;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Durable lifecycle of one non-reassignable external dispatch claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientDispatchState {
    /// Reserved before execution; may already have acted externally.
    Claimed,
    /// A verified result and its tool resolution were committed atomically.
    Completed,
    /// Unknown outcome or cancelled ownership; never permission to redispatch.
    Interrupted,
}

/// Retained authority-owned identity and terminal outcome for one client call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientDispatchRecord {
    /// Full session identity, never a short routing prefix.
    pub session: SessionId,
    /// Exact turn that originally requested the client tool.
    pub turn: TurnId,
    /// Exact retained name, call ID and arguments; no normalization strips fields.
    pub call: ToolCall,
    /// SHA-256 of the exact retained call, without stripping any argument fields.
    pub fingerprint: String,
    /// Host worker incarnation, not an authentication credential.
    pub owner: Uuid,
    /// Immutable claim UUID; never reused for another attempt.
    pub fence: Uuid,
    /// Stable UUID used for both completion and interruption commands.
    pub resolution_id: Uuid,
    /// Authority-clock deadline after which success cannot be committed.
    pub deadline_ms: i64,
    /// Current durable lifecycle state.
    pub state: ClientDispatchState,
    /// Retained terminal result, or none while claimed.
    pub result: Option<ToolCompletedData>,
    /// True only when the terminal tool-resolution event was also committed.
    pub resolution_applied: bool,
}
/// Claim response with non-replayable dispatch permission.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientDispatchClaimReply {
    /// Persisted claim record as observed by this command.
    pub record: ClientDispatchRecord,
    /// Only the first committed claim response permits dispatch. A lost reply
    /// is recovered as interruption, even if the worker never sent the action.
    pub fresh: bool,
}
fn digest_hex(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    Sha256::digest(value)
        .iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 15) as usize] as char,
            ]
        })
        .collect()
}
/// Storage key for a claim, bounded independently of caller-controlled call IDs.
pub fn client_dispatch_key(turn: TurnId, call_id: &str) -> String {
    format!("client_dispatch/{turn}/{}", digest_hex(call_id.as_bytes()))
}
impl SessionCommand {
    /// Records to capture while the authority transaction is held. Include all
    /// pending calls to prevent ordinary resolve commands bypassing a claim.
    pub fn client_dispatch_keys(&self, open: Option<TurnId>, pending: &[ToolCall]) -> Vec<String> {
        if !matches!(
            self,
            Self::BeginClientDispatch { .. }
                | Self::CompleteClientDispatch { .. }
                | Self::InterruptClientDispatch { .. }
                | Self::InspectClientDispatch { .. }
                | Self::Resolve { .. }
                | Self::ResolveWaits { .. }
        ) {
            return Vec::new();
        }
        let mut keys: Vec<_> = open
            .into_iter()
            .flat_map(|turn| {
                pending
                    .iter()
                    .map(move |call| client_dispatch_key(turn, &call.id))
            })
            .collect();
        if let Self::BeginClientDispatch { resolution_id, .. } = self {
            keys.push(format!("client_dispatch_resolution/{resolution_id}"));
        }
        let target = match self {
            Self::BeginClientDispatch { turn, call, .. } => Some((*turn, call.id.as_str())),
            Self::CompleteClientDispatch { turn, call_id, .. }
            | Self::InterruptClientDispatch { turn, call_id, .. }
            | Self::InspectClientDispatch { turn, call_id } => Some((*turn, call_id.as_str())),
            _ => None,
        };
        if let Some((turn, call)) = target {
            keys.push(client_dispatch_key(turn, call));
        }
        keys.sort();
        keys.dedup();
        keys
    }
}
fn pending(
    view: &CommandView,
    turn: TurnId,
    call: &ToolCall,
) -> std::result::Result<EventContext, CommandError> {
    let context = view
        .open
        .clone()
        .filter(|context| context.turn_id == Some(turn) && context.input_message_id.is_some())
        .ok_or_else(|| invalid("client dispatch targets another or closed turn"))?;
    if !view.pending.iter().any(|pending| pending == call)
        || matches!(call.name.as_str(), "approve_tool_call" | "ask_user")
    {
        return Err(invalid(
            "client dispatch does not match an exact pending client call",
        ));
    }
    Ok(context)
}
pub(super) fn prepare_dispatch(
    command: SessionCommand,
    view: &CommandView,
    plan: &mut CommandPlan,
    now: UtcTimestamp,
) -> std::result::Result<(), CommandError> {
    if !view.dispatch_supported {
        return Err(invalid("authority does not support client dispatch"));
    }
    let (turn, call_id) = match &command {
        SessionCommand::BeginClientDispatch { turn, call, .. } => (*turn, call.id.as_str()),
        SessionCommand::CompleteClientDispatch { turn, call_id, .. }
        | SessionCommand::InterruptClientDispatch { turn, call_id, .. }
        | SessionCommand::InspectClientDispatch { turn, call_id } => (*turn, call_id.as_str()),
        _ => return Err(invalid("invalid client dispatch command")),
    };
    let existing = view.dispatches.iter().find(|record| {
        record.session == view.session && record.turn == turn && record.call.id == call_id
    });
    if matches!(command, SessionCommand::InspectClientDispatch { .. }) {
        plan.reply = json!(existing);
        return Ok(());
    }
    match command {
        SessionCommand::BeginClientDispatch {
            id,
            turn,
            call,
            owner,
            resolution_id,
            deadline_ms,
        } => {
            let fence =
                Uuid::parse_str(&id).map_err(|_| invalid("dispatch identity must be a UUID"))?;
            if fence.is_nil()
                || owner.is_nil()
                || resolution_id.is_nil()
                || resolution_id == fence
                || deadline_ms <= view.now_ms
                || deadline_ms.saturating_sub(view.now_ms) > 3_600_000
            {
                return Err(invalid("invalid dispatch owner or deadline"));
            }
            if view
                .dispatches
                .iter()
                .any(|record| record.resolution_id == resolution_id)
            {
                return Err(CommandError::Conflict(
                    "dispatch resolution identity already reserved",
                ));
            }
            if existing.is_some() {
                return Err(CommandError::Conflict(
                    "client call already claimed; never redispatch",
                ));
            }
            pending(view, turn, &call)?;
            let fingerprint = format!(
                "sha256:{}",
                digest_hex(&serde_json::to_vec(&call).map_err(|_| invalid("invalid call"))?)
            );
            let record = ClientDispatchRecord {
                session: view.session,
                turn,
                call,
                fingerprint,
                owner,
                fence,
                resolution_id,
                deadline_ms,
                state: ClientDispatchState::Claimed,
                result: None,
                resolution_applied: false,
            };
            plan.objects.push(ObjectWrite {
                kind: "framework",
                key: format!("client_dispatch_resolution/{resolution_id}"),
                value: json!(record),
            });
            plan.objects.push(ObjectWrite {
                kind: "framework",
                key: client_dispatch_key(turn, &record.call.id),
                value: json!(record),
            });
            plan.reply = json!(ClientDispatchClaimReply {
                record,
                fresh: true
            });
        }
        command => {
            let mut record = existing
                .cloned()
                .ok_or_else(|| invalid("client dispatch claim not found"))?;
            let (id, owner, fence, result, state) = match command {
                SessionCommand::CompleteClientDispatch {
                    id,
                    owner,
                    fence,
                    result,
                    ..
                } => {
                    if owner != record.owner || view.now_ms >= record.deadline_ms {
                        return Err(CommandError::Conflict(
                            "dispatch owner stale or deadline expired",
                        ));
                    }
                    (id, owner, fence, result, ClientDispatchState::Completed)
                }
                SessionCommand::InterruptClientDispatch {
                    id, owner, fence, ..
                } => {
                    if owner != record.owner && view.now_ms < record.deadline_ms {
                        return Err(CommandError::Conflict(
                            "cannot interrupt another live dispatch owner",
                        ));
                    }
                    let result = ToolCompletedData::failure(
                        record.call.id.clone(),
                        record.call.name.clone(),
                        "interrupted".into(),
                        "External client dispatch interrupted; outcome unknown. Do not replay."
                            .into(),
                        None,
                    );
                    (id, owner, fence, result, ClientDispatchState::Interrupted)
                }
                _ => return Err(invalid("invalid dispatch completion")),
            };
            if owner.is_nil()
                || fence != record.fence
                || Uuid::parse_str(&id).ok() != Some(record.resolution_id)
                || record.state != ClientDispatchState::Claimed
            {
                return Err(CommandError::Conflict(
                    "stale or conflicting dispatch completion",
                ));
            }
            let context = match pending(view, record.turn, &record.call) {
                Ok(context) => Some(context),
                Err(_) if state == ClientDispatchState::Interrupted => None,
                Err(error) => return Err(error),
            };
            if result.tool_call_id != record.call.id || result.tool_name != record.call.name {
                return Err(invalid("dispatch result identity mismatch"));
            }
            record.state = state;
            record.result = Some(result.clone());
            if let Some(context) = context {
                validate_tool_resolutions(std::slice::from_ref(&result), &view.pending, now)?;
                plan.events
                    .push(EventRequest::new(view.session, context, result));
                record.resolution_applied = true;
                if view.pending.len() == 1 {
                    plan.control = ControlUpdate::Resume;
                }
            }
            plan.objects.push(ObjectWrite {
                kind: "framework",
                key: client_dispatch_key(record.turn, &record.call.id),
                value: json!(record),
            });
            plan.reply = json!(record);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
