//! A session's authority under Everruns Engine.
//!
//! Engine's `SessionHosting` contract asks one consistency domain to own a
//! session's binding, admitted input, canonical journal, receipts, execution
//! epoch and wakeup obligation together. Here that domain is the cell:
//! every field here is replicated by Raft, and every rule runs in `apply`.
//! The rules follow Everruns' own volatile authority (`MemoryHosting`), with
//! two differences that come from replication:
//!
//! - Time travels in the command (`now_ms`, chosen by the proposer), never
//!   read while applying.
//! - Lease expiry is judged against the proposer's clock in the command, so
//!   every replica fences the same writes. As in Everruns' contract
//!   (`hosting::conformance::lease_expiry`), an expired epoch can no longer
//!   write even before anyone else claims the session.
//!
//! Engine's shared command reducer (`session_commands::prepare`) is pure, so
//! every replica runs it on the same view and gets the same plan. Event ids
//! in a plan are derived from a seed the proposer chose.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use everruns::hosting::SessionBinding;
use everruns::hosting::spi::session_commands::{
    self, CommandReceipt, CommandView, ControlUpdate, SessionCommand,
};
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{EventId, MessageId, TurnId};
use everruns_core::events::{Event, EventContext, EventData, EventRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::types::NodeId;

/// Per-turn receipts kept for this many recent turns.
///
/// Decision: Everruns' authorities keep every receipt; a session of 5,000
/// turns would then hold about 20,000 of them in memory and in every
/// snapshot. Receipts only answer retries and waiters of recent turns, so
/// older ones are dropped: a client that retries an admission more than this
/// many turns later gets a new turn instead of the old receipt.
const KEEP_TURNS: usize = 256;

/// The process running the session now.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Holder {
    /// Engine's holder name (one per activation).
    pub name: String,
    /// The node that took the claim, for liveness and placement.
    pub node: NodeId,
    pub fence: u64,
    /// Proposer's clock: after this the session may be claimed again.
    pub expires_ms: i64,
}

/// Engine's `ProactiveCompactionAttempt`, in a serializable shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProactiveAttempt {
    pub source_sequence: i64,
    pub estimated_input_tokens: u64,
    pub input_message_count: usize,
    pub source_fingerprint: [u8; 32],
}

/// What the cell owns for one session, beside its journal (`SessionState`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Authority {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<SessionBinding>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_at: Option<i64>,
    /// Journal position of the newest admitted input.
    #[serde(default)]
    pub inbox: i64,
    /// Input consumed by turns, as the holder reports it.
    #[serde(default)]
    pub consumed: i64,
    #[serde(default)]
    pub next_input: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<Holder>,
    /// The newest epoch handed out.
    #[serde(default)]
    pub fence: u64,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel: Option<i64>,
    #[serde(default)]
    pub cancel_reported: bool,
    #[serde(default)]
    pub resume_generation: u64,
    #[serde(default)]
    pub resume_claimed: u64,
    #[serde(default)]
    pub runnable: bool,
    #[serde(default)]
    pub suspended: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_input: Option<MessageId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<ToolCall>,
    #[serde(default)]
    pub human_waits: everruns_core::human_wait::HumanWaitState,
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    pub finalizing: HashSet<TurnId>,
    /// Framework receipts (`assignment/`, `outcome/`, `configuration`, ...).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub records: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub pending_inputs: VecDeque<i64>,
    /// Retained command replies, by command key.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub commands: HashMap<String, CommandReceipt>,
    #[serde(default)]
    pub has_history: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub target_cancels: HashMap<MessageId, i64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub terminal: HashMap<TurnId, Event>,
    /// Session values and secrets (Everruns `SessionStorageStore`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secrets: BTreeMap<String, String>,
    /// The runtime's session record (Everruns `RuntimeSessionStore`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<Value>,
    /// Proactive compaction attempts, by `provider/model`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub proactive: BTreeMap<String, ProactiveAttempt>,
    /// Turns finished, oldest first: (turn, its input), for pruning receipts.
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub finished: VecDeque<(TurnId, Option<MessageId>)>,
    /// Turns with a Framework outcome.
    #[serde(default)]
    pub outcomes: u64,
}

impl Authority {
    /// The holder named `name` at `fence`, if it is still the current,
    /// unexpired epoch at `now_ms`.
    pub fn owns(&self, name: &str, fence: u64, now_ms: i64) -> bool {
        self.holder.as_ref().is_some_and(|h| h.name == name) && self.current(fence, now_ms)
    }

    /// Whether epoch `fence` may write at `now_ms`.
    pub fn current(&self, fence: u64, now_ms: i64) -> bool {
        self.holder
            .as_ref()
            .is_some_and(|h| h.fence == fence && h.expires_ms > now_ms)
    }

    /// Whether an execution epoch is leased at `now_ms`.
    pub fn executing(&self, now_ms: i64) -> bool {
        self.holder.as_ref().is_some_and(|h| h.expires_ms > now_ms)
    }

    /// Work is owed: input, a resume, a cancel, or a due timer.
    pub fn owed(&self, now_ms: i64) -> bool {
        !self.suspended && (self.runnable || self.wake_at.is_some_and(|at| at <= now_ms))
    }

    pub fn configuration(&self) -> Value {
        self.records
            .get("configuration")
            .cloned()
            .unwrap_or(json!({}))
    }

    fn note_commit(&mut self, key: &str, value: &Value) {
        if let Some(input) = key
            .strip_prefix("assignment/")
            .and_then(|key| key.parse::<MessageId>().ok())
            && let Some(position) = self.target_cancels.get(&input).copied()
            && let Ok(assignment) =
                serde_json::from_value::<everruns::hosting::TurnAssignment>(value.clone())
            && (self.open.is_none() || self.open == Some(assignment.turn_id))
        {
            self.cancel = Some(position);
            self.cancel_reported = false;
            self.runnable = true;
        }
        if let Some(turn) = key
            .strip_prefix("outcome/")
            .and_then(|key| key.parse::<TurnId>().ok())
        {
            self.finalizing.remove(&turn);
            if !self.records.contains_key(key) {
                self.outcomes += 1;
                let input = self
                    .terminal
                    .get(&turn)
                    .and_then(|e| e.context.input_message_id);
                self.finished.push_back((turn, input));
            }
        }
    }

    /// Drop receipts of turns older than the newest [`KEEP_TURNS`].
    fn prune(&mut self) {
        while self.finished.len() > KEEP_TURNS {
            let Some((turn, input)) = self.finished.pop_front() else {
                break;
            };
            self.terminal.remove(&turn);
            self.records.remove(&format!("outcome/{turn}"));
            self.records.remove(&format!("gate_failure/{turn}"));
            let failures = format!("failure/{turn}/");
            let keys: Vec<String> = self
                .records
                .range(failures.clone()..)
                .take_while(|(k, _)| k.starts_with(&failures))
                .map(|(k, _)| k.clone())
                .collect();
            for key in keys {
                self.records.remove(&key);
            }
            if let Some(input) = input {
                self.records.remove(&format!("turn_for/{input}"));
                self.records.remove(&format!("assignment/{input}"));
                self.commands.remove(&input.to_string());
                self.target_cancels.remove(&input);
            }
        }
    }
}

/// An event id the proposer's `seed` determines, the `n`th of a plan.
fn derived_id(seed: EventId, n: usize) -> EventId {
    EventId::from_uuid(uuid::Uuid::from_u128(
        seed.uuid().as_u128().wrapping_add(n as u128),
    ))
}

/// The outcome of an authority write.
pub enum Applied {
    /// A reply for the proposer (JSON), with the events the write appended.
    Done(Value, Vec<Event>),
    /// The epoch is not the current one.
    Fenced,
    /// Refused, with a reason.
    Refused(String),
    /// A conditional append found the log elsewhere.
    Conflict {
        expected: Option<i32>,
        actual: Option<i32>,
    },
}

impl crate::state::SessionState {
    /// Append one event to the journal, tracking what the authority derives
    /// from events (open turn, pending calls, waits, terminal outcomes).
    fn journal(&mut self, mut request: EventRequest, id: EventId) -> Event {
        self.authority.human_waits.annotate(&mut request);
        let sequence = self.last_sequence().unwrap_or(0) + 1;
        let event = request.into_event(id, sequence);
        let a = &mut self.authority;
        a.human_waits.observe(&event);
        let turn = event.context.turn_id;
        match &event.data {
            EventData::TurnStarted(_) => {
                a.open = turn;
                a.open_input = event.context.input_message_id;
                self.turn_start = sequence;
            }
            EventData::InputMessage(_) | EventData::OutputMessageCompleted(_) => {
                a.has_history = true;
            }
            EventData::ToolCallRequested(data) => a.pending = data.tool_calls.clone(),
            EventData::ToolCompleted(data) => {
                a.pending.retain(|call| call.id != data.tool_call_id);
            }
            _ => {}
        }
        if is_terminal(&event.data)
            && let Some(turn) = turn
        {
            a.open = None;
            a.pending.clear();
            a.finalizing.insert(turn);
            a.terminal.insert(turn, event.clone());
        }
        self.events.push(event.clone());
        event
    }

    /// An executor's append under epoch `fence`.
    pub fn append(
        &mut self,
        fence: u64,
        id: EventId,
        request: EventRequest,
        expected: Option<Option<i32>>,
        now_ms: i64,
    ) -> Applied {
        // A retried proposal (the reply was lost, the command was not)
        // finds its own event at the tail.
        if let Some(event) = self.events.iter().rev().take(32).find(|e| e.id == id) {
            return Applied::Done(json!(event), Vec::new());
        }
        if !self.authority.current(fence, now_ms) {
            return Applied::Fenced;
        }
        if is_terminal(&request.data)
            && let Some(previous) = request
                .context
                .turn_id
                .and_then(|turn| self.authority.terminal.get(&turn))
        {
            if serde_json::to_value(&previous.data).ok() == serde_json::to_value(&request.data).ok()
            {
                return Applied::Done(json!(previous), Vec::new());
            }
            return Applied::Refused("conflicting turn terminal outcome".into());
        }
        let actual = self.last_sequence();
        if let Some(expected) = expected
            && expected != actual
        {
            return Applied::Conflict { expected, actual };
        }
        let event = self.journal(request, id);
        Applied::Done(json!(event), vec![event])
    }

    /// Run one of Engine's session commands (create, admit, cancel, approve,
    /// resolve, configure, ...) through its shared reducer.
    pub fn command(
        &mut self,
        session: everruns_contracts::typed_id::SessionId,
        command: SessionCommand,
        now_ms: i64,
        seed: EventId,
    ) -> Result<(Value, Vec<Event>), String> {
        let a = &mut self.authority;
        while a
            .pending_inputs
            .front()
            .is_some_and(|sequence| *sequence <= a.consumed)
        {
            a.pending_inputs.pop_front();
        }
        let input = if let SessionCommand::Cancel { input, .. } = &command {
            Some(*input)
        } else {
            None
        };
        let assignment = input
            .and_then(|input| a.records.get(&format!("assignment/{input}")))
            .and_then(|value| {
                serde_json::from_value::<everruns::hosting::TurnAssignment>(value.clone()).ok()
            })
            .map(|a| a.turn_id);
        let head = self.last_sequence().unwrap_or(0);
        let a = &self.authority;
        let view = CommandView {
            session,
            now_ms,
            binding: a.binding.as_ref().map(|value| json!(value)),
            execution_active: a.executing(now_ms),
            suspended: a.suspended,
            suspension_reason: a
                .records
                .get("suspension")
                .and_then(|value| value["reason"].as_str().map(str::to_owned)),
            head,
            open: a
                .open
                .zip(a.open_input)
                .map(|(turn, input)| EventContext::turn(turn, input)),
            pending: a.pending.clone(),
            configuration: serde_json::from_value(a.configuration()).map_err(|e| e.to_string())?,
            previous: command.key().and_then(|key| a.commands.get(&key).cloned()),
            command_count: a.commands.len(),
            queued: a.pending_inputs.len(),
            has_history: a.has_history,
            admitted: input.is_some_and(|input| a.commands.contains_key(&input.to_string())),
            assignment,
            terminal: assignment.is_some_and(|turn| a.terminal.contains_key(&turn)),
            rejected: input
                .is_some_and(|input| a.records.contains_key(&format!("rejected/{input}"))),
            // No branch facts: this authority refuses branch selection.
            history: None,
        };
        let plan = session_commands::prepare(command, view).map_err(|e| e.to_string())?;
        if matches!(
            plan.control,
            ControlUpdate::SelectBranch { .. } | ControlUpdate::DropBranch { .. }
        ) {
            return Err(
                "this session authority does not support conversation branch selection".into(),
            );
        }
        if plan.replay {
            return Ok((plan.reply, Vec::new()));
        }
        let binding = if let ControlUpdate::Bind { binding } = &plan.control {
            Some(
                serde_json::from_value::<SessionBinding>(binding.clone())
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        for object in &plan.objects {
            if object.kind == "value" && object.value.as_str().is_none() {
                return Err("invalid shared value write".into());
            }
        }
        for object in plan.objects {
            if object.kind == "value" {
                self.authority.values.insert(
                    object.key,
                    object.value.as_str().unwrap_or_default().to_string(),
                );
            } else {
                self.authority.records.insert(object.key, object.value);
            }
        }
        let mut head = head;
        let mut appended = Vec::new();
        for (n, event) in plan.events.into_iter().enumerate() {
            let event = self.journal(event, derived_id(seed, n));
            head = event.sequence.unwrap_or(head);
            appended.push(event);
        }
        let a = &mut self.authority;
        match plan.control {
            ControlUpdate::None
            | ControlUpdate::SelectBranch { .. }
            | ControlUpdate::DropBranch { .. } => {}
            ControlUpdate::Bind { .. } => {
                if a.binding.is_none() {
                    a.created_at = chrono::DateTime::from_timestamp_millis(now_ms)
                        .unwrap_or_default()
                        .to_rfc3339();
                }
                a.binding = binding;
            }
            ControlUpdate::Admit { message, cancelled } => {
                a.pending_inputs.push_back(i64::from(head));
                if cancelled {
                    a.target_cancels.insert(message, i64::from(head));
                }
                a.inbox = i64::from(head);
                if a.next_input <= a.consumed {
                    a.next_input = a.inbox;
                }
                a.runnable = true;
            }
            ControlUpdate::Resume => {
                a.inbox = i64::from(head);
                a.resume_generation += 1;
                a.runnable = true;
            }
            ControlUpdate::RetryExecution => {
                a.suspended = false;
                a.runnable = true;
                a.holder = None;
            }
            ControlUpdate::Cancel { input, interrupt } => {
                let position = a.inbox;
                a.target_cancels.insert(input, position);
                if interrupt {
                    a.cancel = Some(position);
                    a.cancel_reported = false;
                    a.runnable = true;
                }
            }
        }
        if let Some(key) = plan.key {
            a.commands.insert(
                key,
                CommandReceipt {
                    fingerprint: plan.fingerprint,
                    value: plan.reply.clone(),
                },
            );
        }
        Ok((plan.reply, appended))
    }

    /// Take a new epoch for `name` on `node`, if the session owes work and
    /// no unexpired epoch holds it. The reply is Engine's `LeasedActor`.
    pub fn claim(&mut self, name: &str, node: NodeId, ttl_ms: i64, now_ms: i64) -> Option<Value> {
        let a = &mut self.authority;
        a.binding.as_ref()?;
        if a.wake_at.is_some_and(|at| at <= now_ms) {
            a.wake_at = None;
            a.resume_generation += 1;
            a.runnable = true;
        }
        if !a.runnable || a.suspended || a.executing(now_ms) {
            return None;
        }
        a.fence += 1;
        a.holder = Some(Holder {
            name: name.to_owned(),
            node,
            fence: a.fence,
            expires_ms: now_ms + ttl_ms,
        });
        a.attempts += 1;
        a.cancel_reported = a.cancel.is_some();
        a.resume_claimed = a.resume_generation;
        Some(json!({
            "fence": a.fence,
            "inbox_seq": a.inbox,
            "cancel_inbox_seq": a.cancel,
            "consumed_seq": a.consumed,
            "next_input_seq": a.next_input,
            "attempts": a.attempts,
        }))
    }

    /// Commit receipts that ride on a lease write (Engine's
    /// `commit_with_lease`), each only if absent, under the lease write's
    /// own ownership check: they commit exactly when it does.
    fn ride(&mut self, receipts: Vec<(String, Value)>) {
        let a = &mut self.authority;
        for (key, value) in receipts {
            if !a.records.contains_key(&key) {
                a.note_commit(&key, &value);
                a.records.insert(key, value);
            }
        }
    }

    /// Extend `name`'s epoch `fence`, committing `receipts` with it. `None`:
    /// the epoch was superseded.
    pub fn renew(
        &mut self,
        name: &str,
        fence: u64,
        ttl_ms: i64,
        consumed: i64,
        receipts: Vec<(String, Value)>,
        now_ms: i64,
    ) -> Option<Value> {
        if !self.authority.owns(name, fence, now_ms) {
            return None;
        }
        self.ride(receipts);
        let a = &mut self.authority;
        a.consumed = a.consumed.max(consumed);
        if let Some(holder) = a.holder.as_mut() {
            holder.expires_ms = holder.expires_ms.max(now_ms + ttl_ms);
        }
        a.cancel_reported = a.cancel.is_some();
        Some(json!({ "inbox_seq": a.inbox, "cancel_inbox_seq": a.cancel }))
    }

    /// `name`'s resumed epoch `fence` made progress: count attempts from it
    /// again. `false`: the epoch was superseded.
    pub fn progress(&mut self, name: &str, fence: u64, now_ms: i64) -> bool {
        let a = &mut self.authority;
        if !a.owns(name, fence, now_ms) {
            return false;
        }
        a.attempts = 1;
        true
    }

    /// Give `name`'s epoch back. With `hand_off`, mid-turn: the session stays
    /// runnable so the next epoch resumes the open turn at once.
    pub fn release(
        &mut self,
        name: &str,
        fence: u64,
        consumed: i64,
        hand_off: bool,
        receipts: Vec<(String, Value)>,
        now_ms: i64,
    ) -> Option<bool> {
        if !self.authority.owns(name, fence, now_ms) {
            return None;
        }
        self.ride(receipts);
        let a = &mut self.authority;
        a.consumed = a.consumed.max(consumed);
        a.holder = None;
        a.attempts = 0;
        a.runnable = hand_off
            || !a.finalizing.is_empty()
            || a.resume_generation > a.resume_claimed
            || a.inbox > a.consumed
            || (a.cancel.is_some() && !a.cancel_reported);
        if a.cancel_reported {
            a.cancel = None;
        }
        a.cancel_reported = false;
        a.prune();
        Some(a.runnable)
    }

    /// A Framework write under epoch `fence`: a receipt, suspension, timer,
    /// session value, session record or compaction state.
    pub fn write(&mut self, fence: u64, op: WriteOp, now_ms: i64) -> Applied {
        if !self.authority.current(fence, now_ms) {
            return Applied::Fenced;
        }
        let a = &mut self.authority;
        let reply = match op {
            WriteOp::Commit { key, value, absent } => {
                if absent && a.records.contains_key(&key) {
                    json!(false)
                } else {
                    a.note_commit(&key, &value);
                    a.records.insert(key, value);
                    json!(true)
                }
            }
            WriteOp::Suspend { reason } => {
                a.suspended = true;
                a.records
                    .insert("suspension".into(), json!({ "reason": reason }));
                Value::Null
            }
            WriteOp::Schedule { wake_at } => {
                a.wake_at = wake_at;
                Value::Null
            }
            WriteOp::SetValue { key, value } => {
                a.values.insert(key, value);
                Value::Null
            }
            WriteOp::TakeValue { key } => json!(a.values.remove(&key)),
            WriteOp::DeleteValue { key } => json!(a.values.remove(&key).is_some()),
            WriteOp::SetSecret { key, value } => {
                a.secrets.insert(key, value);
                Value::Null
            }
            WriteOp::DeleteSecret { key } => json!(a.secrets.remove(&key).is_some()),
            WriteOp::Execution { session } => {
                a.execution = Some(session);
                Value::Null
            }
            WriteOp::Title { title } => match a.execution.as_mut() {
                Some(session) => {
                    session["title"] = json!(title);
                    session.clone()
                }
                None => return Applied::Refused("no session record".into()),
            },
            WriteOp::Checkpoint { checkpoint } => {
                let same = |c: &crate::state::Checkpoint| {
                    c.provider_type == checkpoint.provider_type
                        && c.model == checkpoint.model
                        && c.format_version == checkpoint.format_version
                };
                match self.checkpoints.iter_mut().find(|c| same(c)) {
                    Some(current) if current.source_sequence >= checkpoint.source_sequence => {
                        json!(false)
                    }
                    Some(current) => {
                        *current = checkpoint;
                        json!(true)
                    }
                    None => {
                        self.checkpoints.push(checkpoint);
                        json!(true)
                    }
                }
            }
            WriteOp::Proactive { key, attempt } => {
                a.proactive.insert(key, attempt);
                Value::Null
            }
        };
        Applied::Done(reply, Vec::new())
    }
}

/// Framework writes fenced by an epoch (see [`SessionState::write`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "write", rename_all = "snake_case")]
pub enum WriteOp {
    Commit {
        key: String,
        value: Value,
        absent: bool,
    },
    Suspend {
        reason: String,
    },
    Schedule {
        wake_at: Option<i64>,
    },
    SetValue {
        key: String,
        value: String,
    },
    TakeValue {
        key: String,
    },
    DeleteValue {
        key: String,
    },
    SetSecret {
        key: String,
        value: String,
    },
    DeleteSecret {
        key: String,
    },
    Execution {
        session: Value,
    },
    Title {
        title: String,
    },
    Checkpoint {
        checkpoint: crate::state::Checkpoint,
    },
    Proactive {
        key: String,
        attempt: ProactiveAttempt,
    },
}

pub fn is_terminal(data: &EventData) -> bool {
    matches!(
        data,
        EventData::TurnCompleted(_)
            | EventData::TurnFailed(_)
            | EventData::TurnCancelled(_)
            | EventData::TurnSealed(_)
    )
}
