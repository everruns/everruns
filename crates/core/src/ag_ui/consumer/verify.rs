//! Step two of the consumer pipeline: chunk expansion and the 1.0
//! sequencing rules.
//!
//! Ported from the reference TypeScript client (`chunks/transform.ts` and
//! `verify/verify.ts` in ag-ui-protocol/ag-ui), which the upstream
//! conformance corpus in `spec/1.0/conformance` is written against. Error
//! messages keep the reference wording where it is the rule's name, so a
//! failure reads the same in both clients.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::ProtocolError;
use crate::ag_ui::{
    BaseEvent, Event, Message, ReasoningEncryptedValueSubtype, ReasoningMessageContentEvent,
    ReasoningMessageEndEvent, ReasoningMessageStartEvent, TextMessageContentEvent,
    TextMessageEndEvent, TextMessageRole, TextMessageStartEvent, ToolCallArgsEvent,
    ToolCallEndEvent, ToolCallStartEvent,
};

/// Who produced an entity: `None` is the parent agent, `Some` a subagent.
type Owner = Option<String>;

fn owner_label(owner: &Owner) -> &str {
    owner.as_deref().unwrap_or("(the parent agent)")
}

/// The stream a lane is assembling from `*_CHUNK` events.
#[derive(Clone, Debug)]
enum Pending {
    Text {
        message_id: String,
        role: TextMessageRole,
        name: Option<String>,
        owner: Owner,
    },
    Tool {
        tool_call_id: String,
        tool_call_name: String,
        parent_message_id: Option<String>,
        owner: Owner,
    },
    Reasoning {
        message_id: String,
        owner: Owner,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChunkKind {
    Text,
    Tool,
    Reasoning,
}

impl Pending {
    fn kind(&self) -> ChunkKind {
        match self {
            Self::Text { .. } => ChunkKind::Text,
            Self::Tool { .. } => ChunkKind::Tool,
            Self::Reasoning { .. } => ChunkKind::Reasoning,
        }
    }

    fn entity_id(&self) -> &str {
        match self {
            Self::Text { message_id, .. } | Self::Reasoning { message_id, .. } => message_id,
            Self::Tool { tool_call_id, .. } => tool_call_id,
        }
    }

    /// The `*_END` that closes this stream.
    fn close(self) -> Event {
        match self {
            Self::Text {
                message_id, owner, ..
            } => Event::TextMessageEnd(TextMessageEndEvent {
                subagent_run_id: owner,
                ..TextMessageEndEvent::new(message_id)
            }),
            Self::Tool {
                tool_call_id,
                owner,
                ..
            } => Event::ToolCallEnd(ToolCallEndEvent {
                subagent_run_id: owner,
                ..ToolCallEndEvent::new(tool_call_id)
            }),
            Self::Reasoning { message_id, owner } => {
                Event::ReasoningMessageEnd(ReasoningMessageEndEvent {
                    subagent_run_id: owner,
                    ..ReasoningMessageEndEvent::new(message_id)
                })
            }
        }
    }
}

/// A continuation chunk may repeat a field its opener set, with the same
/// value only.
fn require_agreement(
    entity: &str,
    id: &str,
    field: &str,
    incoming: Option<&str>,
    established: Option<&str>,
) -> Result<(), ProtocolError> {
    match incoming {
        Some(value) if Some(value) != established => Err(ProtocolError::new(format!(
            "Cannot continue {entity} '{id}': chunk {field} '{value}' does not match the open stream's {field} {}.",
            established.map_or_else(|| "(absent)".to_owned(), |v| format!("'{v}'"))
        ))),
        _ => Ok(()),
    }
}

/// The 1.0 consumer rules over a typed event stream.
///
/// Feed it events in arrival order with [`push`](Self::push); it expands the
/// `*_CHUNK` shorthand into start/content/end triads and checks every event
/// against the sequencing rules, returning the events an application should
/// apply. The first violation is an error, after which the stream should be
/// abandoned.
///
/// One stream may carry several runs in sequence: a `RUN_STARTED` after a
/// `RUN_FINISHED` or `RUN_ERROR` begins a new one.
///
/// ```
/// use everruns_core::ag_ui::consumer::Verifier;
/// use everruns_core::ag_ui::{Event, RunStartedEvent, TextMessageChunkEvent};
///
/// let mut verifier = Verifier::new();
/// verifier.push(Event::RunStarted(RunStartedEvent::new("t", "r"))).unwrap();
///
/// // A chunk expands into the start and content events it stands for.
/// let expanded = verifier
///     .push(Event::TextMessageChunk(TextMessageChunkEvent {
///         message_id: Some("m1".into()),
///         delta: Some("Hi".into()),
///         ..Default::default()
///     }))
///     .unwrap();
/// assert_eq!(expanded.len(), 2);
///
/// // Content for a message nobody opened is a violation.
/// let orphan = Event::TextMessageContent(everruns_core::ag_ui::TextMessageContentEvent::new("m9", "?"));
/// assert!(verifier.push(orphan).is_err());
/// ```
#[derive(Debug, Default)]
pub struct Verifier {
    first_event_seen: bool,
    run_started: bool,
    run_finished: bool,
    run_errored: bool,
    active_messages: HashSet<String>,
    active_tool_calls: HashSet<String>,
    active_reasoning_spans: HashSet<String>,
    active_reasoning_messages: HashSet<String>,
    // Owners outlive the close: a continuation (an encrypted value, a reopen)
    // can arrive after its entity closed and must still agree with it.
    message_owners: HashMap<String, Owner>,
    tool_call_owners: HashMap<String, Owner>,
    activity_owners: HashMap<String, Owner>,
    reasoning_owners: HashMap<String, Owner>,
    // Steps are keyed by owner: a subagent may run a step of the same name
    // as its parent at the same time.
    active_steps: BTreeMap<Owner, BTreeSet<String>>,
    active_subagents: Vec<String>,
    closed_subagents: HashSet<String>,
    // One chunk stream per lane (owner), in the order the lanes opened.
    lanes: Vec<(Owner, Pending)>,
}

impl Verifier {
    /// A verifier for a stream that has not started.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a run is open: started and not yet finished or errored.
    pub fn run_active(&self) -> bool {
        self.run_started && !self.run_finished && !self.run_errored
    }

    /// Whether any event has been seen.
    pub fn started(&self) -> bool {
        self.first_event_seen
    }

    /// Expands and verifies one event, returning what the application should
    /// apply, in order (empty for a chunk carrying nothing to apply).
    pub fn push(&mut self, event: Event) -> Result<Vec<Event>, ProtocolError> {
        let expanded = self.expand(event)?;
        for event in &expanded {
            self.verify(event)?;
        }
        Ok(expanded)
    }

    /// Checks the end of the stream: a stream must not stop inside a run.
    pub fn finish(&mut self) -> Result<(), ProtocolError> {
        if !self.first_event_seen {
            return Err(ProtocolError::new("the stream ended without any event"));
        }
        if self.run_active() {
            return Err(ProtocolError::new(
                "the stream ended before the run finished with 'RUN_FINISHED' or 'RUN_ERROR'",
            ));
        }
        Ok(())
    }

    // ----- chunk expansion -------------------------------------------------

    fn close_lane(&mut self, owner: &Owner) -> Option<Event> {
        let index = self.lanes.iter().position(|(o, _)| o == owner)?;
        let (_, pending) = self.lanes.remove(index);
        Some(pending.close())
    }

    fn close_all_lanes(&mut self) -> Vec<Event> {
        self.lanes
            .drain(..)
            .map(|(_, pending)| pending.close())
            .collect()
    }

    fn lane(&self, owner: &Owner) -> Option<&Pending> {
        self.lanes
            .iter()
            .find(|(o, _)| o == owner)
            .map(|(_, pending)| pending)
    }

    /// Which lane a chunk continues or opens. An id names its lane outright;
    /// an id-less continuation falls back to its tag, then the parent's open
    /// stream, then the sole open stream of its kind.
    fn resolve_lane(
        &self,
        kind: ChunkKind,
        entity_id: Option<&str>,
        tag: Option<&str>,
        chunk_type: &str,
        entity: &str,
    ) -> Result<Owner, ProtocolError> {
        if let Some(id) = entity_id {
            let holder = self
                .lanes
                .iter()
                .find(|(_, p)| p.kind() == kind && p.entity_id() == id);
            if let Some((owner, _)) = holder {
                if let Some(tag) = tag
                    && Some(tag) != owner.as_deref()
                {
                    return Err(ProtocolError::new(format!(
                        "Cannot continue {entity} '{id}': chunk subagentRunId '{tag}' does not match the open stream's subagent '{}'.",
                        owner_label(owner)
                    )));
                }
                return Ok(owner.clone());
            }
            return Ok(tag.map(str::to_owned));
        }
        if let Some(tag) = tag {
            return Ok(Some(tag.to_owned()));
        }
        if self.lane(&None).is_some_and(|p| p.kind() == kind) {
            return Ok(None);
        }
        let candidates: Vec<&Owner> = self
            .lanes
            .iter()
            .filter(|(_, p)| p.kind() == kind)
            .map(|(owner, _)| owner)
            .collect();
        match candidates.as_slice() {
            [] => Ok(None),
            [only] => Ok((*only).clone()),
            many => Err(ProtocolError::new(format!(
                "Ambiguous {chunk_type}: it carries no id and no subagentRunId, but {} lanes have an open {entity}.",
                many.len()
            ))),
        }
    }

    fn expand(&mut self, event: Event) -> Result<Vec<Event>, ProtocolError> {
        let mut out = Vec::new();
        match event {
            Event::TextMessageChunk(chunk) => {
                let lane = self.resolve_lane(
                    ChunkKind::Text,
                    chunk.message_id.as_deref(),
                    chunk.subagent_run_id.as_deref(),
                    "TEXT_MESSAGE_CHUNK",
                    "text message",
                )?;
                let continuing = match self.lane(&lane) {
                    Some(Pending::Text {
                        message_id,
                        role,
                        name,
                        owner,
                    }) if chunk.message_id.as_ref().is_none_or(|id| id == message_id) => {
                        let role_text = role_str(*role);
                        let incoming_role = chunk.role.map(role_str);
                        require_agreement(
                            "text message",
                            message_id,
                            "role",
                            incoming_role,
                            Some(role_text),
                        )?;
                        require_agreement(
                            "text message",
                            message_id,
                            "name",
                            chunk.name.as_deref(),
                            name.as_deref(),
                        )?;
                        Some((message_id.clone(), owner.clone()))
                    }
                    _ => None,
                };
                let (message_id, opener_owner) = match continuing {
                    Some(found) => found,
                    None => {
                        out.extend(self.close_lane(&lane));
                        let Some(message_id) = chunk.message_id.clone() else {
                            return Err(ProtocolError::new(
                                "First TEXT_MESSAGE_CHUNK must have a messageId",
                            ));
                        };
                        let role = chunk.role.unwrap_or(TextMessageRole::Assistant);
                        self.lanes.push((
                            lane,
                            Pending::Text {
                                message_id: message_id.clone(),
                                role,
                                name: chunk.name.clone(),
                                owner: chunk.subagent_run_id.clone(),
                            },
                        ));
                        out.push(Event::TextMessageStart(TextMessageStartEvent {
                            base: chunk_opener_base(&chunk.base),
                            message_id: message_id.clone(),
                            role,
                            name: chunk.name.clone(),
                            subagent_run_id: chunk.subagent_run_id.clone(),
                        }));
                        (message_id, chunk.subagent_run_id.clone())
                    }
                };
                let owner = chunk.subagent_run_id.clone().or(opener_owner);
                if emits_content(chunk.delta.is_some(), &chunk.base, out.is_empty()) {
                    out.push(Event::TextMessageContent(TextMessageContentEvent {
                        base: chunk.base,
                        message_id,
                        delta: chunk.delta.unwrap_or_default(),
                        subagent_run_id: owner,
                    }));
                }
            }
            Event::ToolCallChunk(chunk) => {
                let lane = self.resolve_lane(
                    ChunkKind::Tool,
                    chunk.tool_call_id.as_deref(),
                    chunk.subagent_run_id.as_deref(),
                    "TOOL_CALL_CHUNK",
                    "tool call",
                )?;
                let continuing = match self.lane(&lane) {
                    Some(Pending::Tool {
                        tool_call_id,
                        tool_call_name,
                        parent_message_id,
                        owner,
                    }) if chunk
                        .tool_call_id
                        .as_ref()
                        .is_none_or(|id| id == tool_call_id) =>
                    {
                        require_agreement(
                            "tool call",
                            tool_call_id,
                            "toolCallName",
                            chunk.tool_call_name.as_deref(),
                            Some(tool_call_name),
                        )?;
                        require_agreement(
                            "tool call",
                            tool_call_id,
                            "parentMessageId",
                            chunk.parent_message_id.as_deref(),
                            parent_message_id.as_deref(),
                        )?;
                        Some((tool_call_id.clone(), owner.clone()))
                    }
                    _ => None,
                };
                let (tool_call_id, opener_owner) = match continuing {
                    Some(found) => found,
                    None => {
                        out.extend(self.close_lane(&lane));
                        let Some(tool_call_id) = chunk.tool_call_id.clone() else {
                            return Err(ProtocolError::new(
                                "First TOOL_CALL_CHUNK must have a toolCallId",
                            ));
                        };
                        let Some(tool_call_name) = chunk.tool_call_name.clone() else {
                            return Err(ProtocolError::new(
                                "First TOOL_CALL_CHUNK must have a toolCallName",
                            ));
                        };
                        self.lanes.push((
                            lane,
                            Pending::Tool {
                                tool_call_id: tool_call_id.clone(),
                                tool_call_name: tool_call_name.clone(),
                                parent_message_id: chunk.parent_message_id.clone(),
                                owner: chunk.subagent_run_id.clone(),
                            },
                        ));
                        out.push(Event::ToolCallStart(ToolCallStartEvent {
                            base: chunk_opener_base(&chunk.base),
                            tool_call_id: tool_call_id.clone(),
                            tool_call_name,
                            parent_message_id: chunk.parent_message_id.clone(),
                            subagent_run_id: chunk.subagent_run_id.clone(),
                        }));
                        (tool_call_id, chunk.subagent_run_id.clone())
                    }
                };
                let owner = chunk.subagent_run_id.clone().or(opener_owner);
                if emits_content(chunk.delta.is_some(), &chunk.base, out.is_empty()) {
                    out.push(Event::ToolCallArgs(ToolCallArgsEvent {
                        base: chunk.base,
                        tool_call_id,
                        delta: chunk.delta.unwrap_or_default(),
                        subagent_run_id: owner,
                    }));
                }
            }
            Event::ReasoningMessageChunk(chunk) => {
                let lane = self.resolve_lane(
                    ChunkKind::Reasoning,
                    chunk.message_id.as_deref(),
                    chunk.subagent_run_id.as_deref(),
                    "REASONING_MESSAGE_CHUNK",
                    "reasoning message",
                )?;
                let continuing = match self.lane(&lane) {
                    Some(Pending::Reasoning { message_id, owner })
                        if chunk.message_id.as_ref().is_none_or(|id| id == message_id) =>
                    {
                        Some((message_id.clone(), owner.clone()))
                    }
                    _ => None,
                };
                let (message_id, opener_owner) = match continuing {
                    Some(found) => found,
                    None => {
                        out.extend(self.close_lane(&lane));
                        let Some(message_id) = chunk.message_id.clone() else {
                            return Err(ProtocolError::new(
                                "First REASONING_MESSAGE_CHUNK must have a messageId",
                            ));
                        };
                        self.lanes.push((
                            lane,
                            Pending::Reasoning {
                                message_id: message_id.clone(),
                                owner: chunk.subagent_run_id.clone(),
                            },
                        ));
                        out.push(Event::ReasoningMessageStart(ReasoningMessageStartEvent {
                            base: chunk_opener_base(&chunk.base),
                            subagent_run_id: chunk.subagent_run_id.clone(),
                            ..ReasoningMessageStartEvent::new(message_id.clone())
                        }));
                        (message_id, chunk.subagent_run_id.clone())
                    }
                };
                let owner = chunk.subagent_run_id.clone().or(opener_owner);
                if emits_content(chunk.delta.is_some(), &chunk.base, out.is_empty()) {
                    out.push(Event::ReasoningMessageContent(
                        ReasoningMessageContentEvent {
                            base: chunk.base,
                            message_id,
                            delta: chunk.delta.unwrap_or_default(),
                            subagent_run_id: owner,
                        },
                    ));
                }
            }
            // Run-level events describe the run as a whole: every lane closes.
            Event::RunStarted(_)
            | Event::RunFinished(_)
            | Event::RunError(_)
            | Event::MessagesSnapshot(_) => {
                out.extend(self.close_all_lanes());
                out.push(event);
            }
            // These stand aside from chunk assembly.
            Event::Raw(_)
            | Event::ActivitySnapshot(_)
            | Event::ActivityDelta(_)
            | Event::ReasoningEncryptedValue(_)
            | Event::SubagentStarted(_) => out.push(event),
            // A subagent's terminal closes its own lane only.
            Event::SubagentFinished(ref e) => {
                let lane = Some(e.subagent_run_id.clone());
                out.extend(self.close_lane(&lane));
                out.push(event);
            }
            Event::SubagentError(ref e) => {
                let lane = Some(e.subagent_run_id.clone());
                out.extend(self.close_lane(&lane));
                out.push(event);
            }
            // Any other explicit event closes the stream open in its own lane.
            other => {
                let lane = attribution(&other).map(str::to_owned);
                out.extend(self.close_lane(&lane));
                out.push(other);
            }
        }
        Ok(out)
    }

    // ----- verification ----------------------------------------------------

    fn reset_run(&mut self) {
        self.active_messages.clear();
        self.active_tool_calls.clear();
        self.active_reasoning_spans.clear();
        self.active_reasoning_messages.clear();
        self.message_owners.clear();
        self.tool_call_owners.clear();
        self.activity_owners.clear();
        self.reasoning_owners.clear();
        self.active_steps.clear();
        self.active_subagents.clear();
        self.closed_subagents.clear();
        self.run_finished = false;
        self.run_errored = false;
        self.run_started = true;
    }

    /// Seeds owners from replayed history. A snapshot is authoritative and
    /// replaces recorded owners; a `RUN_STARTED` input echo only fills gaps.
    fn seed_owners(&mut self, messages: &[Message], authoritative: bool) {
        for message in messages {
            let (bucket, owner) = match message {
                Message::Reasoning(m) => (&mut self.reasoning_owners, m.subagent_run_id.clone()),
                Message::Activity(m) => (&mut self.activity_owners, m.subagent_run_id.clone()),
                Message::Assistant(m) => (&mut self.message_owners, m.subagent_run_id.clone()),
                Message::User(m) => (&mut self.message_owners, m.subagent_run_id.clone()),
                Message::Tool(m) => (&mut self.message_owners, m.subagent_run_id.clone()),
                Message::Developer(m) | Message::System(m) => {
                    (&mut self.message_owners, m.subagent_run_id.clone())
                }
            };
            if authoritative || !bucket.contains_key(message.id()) {
                bucket.insert(message.id().to_owned(), owner.clone());
            }
            if let Message::Assistant(m) = message {
                for call in m.tool_calls.iter().flatten() {
                    if authoritative || !self.tool_call_owners.contains_key(&call.id) {
                        self.tool_call_owners.insert(call.id.clone(), owner.clone());
                    }
                }
            }
        }
    }

    fn verify(&mut self, event: &Event) -> Result<(), ProtocolError> {
        let event_type = event.event_type();
        let tag = attribution(event);

        if self.run_errored && !matches!(event, Event::RunStarted(_)) {
            return Err(ProtocolError::new(format!(
                "Cannot send event type '{event_type}': The run has already errored with 'RUN_ERROR'. No further events can be sent."
            )));
        }
        if self.run_finished && !matches!(event, Event::RunError(_) | Event::RunStarted(_)) {
            return Err(ProtocolError::new(format!(
                "Cannot send event type '{event_type}': The run has already finished with 'RUN_FINISHED'. Start a new run with 'RUN_STARTED'."
            )));
        }
        if !self.first_event_seen {
            self.first_event_seen = true;
            if !matches!(event, Event::RunStarted(_) | Event::RunError(_)) {
                return Err(ProtocolError::new("First event must be 'RUN_STARTED'"));
            }
        } else if matches!(event, Event::RunStarted(_)) {
            if self.run_active() {
                return Err(ProtocolError::new(
                    "Cannot send 'RUN_STARTED' while a run is still active. The previous run must be finished with 'RUN_FINISHED' before starting a new run.",
                ));
            }
            if self.run_finished || self.run_errored {
                self.reset_run();
            }
        }

        match event {
            Event::TextMessageStart(e) => {
                let id = &e.message_id;
                if self.active_messages.contains(id) {
                    return Err(ProtocolError::new(format!(
                        "Cannot send 'TEXT_MESSAGE_START' event: A text message with ID '{id}' is already in progress. Complete it with 'TEXT_MESSAGE_END' first."
                    )));
                }
                // First writer wins: a reopen must agree with the opener.
                match self.message_owners.get(id) {
                    Some(owner) => check_owner(event_type, tag, owner, "message", id)?,
                    None => {
                        self.message_owners
                            .insert(id.clone(), e.subagent_run_id.clone());
                    }
                }
                self.active_messages.insert(id.clone());
            }
            Event::TextMessageContent(e) => {
                self.continue_message(
                    event_type,
                    tag,
                    &e.message_id,
                    "Start a text message with 'TEXT_MESSAGE_START' first.",
                )?;
            }
            Event::TextMessageEnd(e) => {
                self.continue_message(
                    event_type,
                    tag,
                    &e.message_id,
                    "A 'TEXT_MESSAGE_START' event must be sent first.",
                )?;
                self.active_messages.remove(&e.message_id);
            }
            Event::ToolCallStart(e) => {
                let id = &e.tool_call_id;
                if self.active_tool_calls.contains(id) {
                    return Err(ProtocolError::new(format!(
                        "Cannot send 'TOOL_CALL_START' event: A tool call with ID '{id}' is already in progress. Complete it with 'TOOL_CALL_END' first."
                    )));
                }
                // A call lives inside its parent message, so it inherits the
                // message's owner and may not contradict it.
                let mut inherited: Option<Owner> = None;
                if let Some(parent) = &e.parent_message_id
                    && let Some(parent_owner) = self.message_owners.get(parent)
                {
                    if let Some(tag) = tag
                        && Some(tag) != parent_owner.as_deref()
                    {
                        return Err(ProtocolError::new(format!(
                            "Cannot send 'TOOL_CALL_START': subagentRunId '{tag}' does not match its parent message '{parent}' owner '{}'. A tool call belongs to the message that carries it.",
                            owner_label(parent_owner)
                        )));
                    }
                    inherited = Some(parent_owner.clone());
                }
                match self.tool_call_owners.get(id) {
                    Some(existing) => {
                        check_owner(event_type, tag, existing, "tool call", id)?;
                        if tag.is_none()
                            && let Some(inherited) = &inherited
                            && inherited != existing
                        {
                            return Err(ProtocolError::new(format!(
                                "Cannot send 'TOOL_CALL_START': tool call '{id}' is owned by '{}' but its parent message is owned by '{}'. A tool call belongs to the message that carries it.",
                                owner_label(existing),
                                owner_label(inherited)
                            )));
                        }
                    }
                    None => {
                        let owner = match tag {
                            Some(tag) => Some(tag.to_owned()),
                            None => inherited.unwrap_or_default(),
                        };
                        self.tool_call_owners.insert(id.clone(), owner);
                    }
                }
                self.active_tool_calls.insert(id.clone());
            }
            Event::ToolCallArgs(e) => {
                self.continue_tool_call(
                    event_type,
                    tag,
                    &e.tool_call_id,
                    "Start a tool call with 'TOOL_CALL_START' first.",
                )?;
            }
            Event::ToolCallEnd(e) => {
                self.continue_tool_call(
                    event_type,
                    tag,
                    &e.tool_call_id,
                    "A 'TOOL_CALL_START' event must be sent first.",
                )?;
                self.active_tool_calls.remove(&e.tool_call_id);
            }
            Event::ToolCallResult(e) => {
                // Mints a tool message; the newest mint owns the id.
                self.message_owners
                    .insert(e.message_id.clone(), e.subagent_run_id.clone());
            }
            Event::StepStarted(e) => {
                let steps = self
                    .active_steps
                    .entry(e.subagent_run_id.clone())
                    .or_default();
                if !steps.insert(e.step_name.clone()) {
                    return Err(ProtocolError::new(format!(
                        "Step \"{}\" is already active for 'STEP_STARTED'{}",
                        e.step_name,
                        e.subagent_run_id
                            .as_deref()
                            .map(|s| format!(" in subagent '{s}'"))
                            .unwrap_or_default()
                    )));
                }
            }
            Event::StepFinished(e) => {
                let owner = &e.subagent_run_id;
                let removed = self
                    .active_steps
                    .get_mut(owner)
                    .is_some_and(|steps| steps.remove(&e.step_name));
                if !removed {
                    let elsewhere = self
                        .active_steps
                        .iter()
                        .find(|(o, steps)| *o != owner && steps.contains(&e.step_name));
                    return Err(ProtocolError::new(match elsewhere {
                        Some((other, _)) => format!(
                            "Cannot send 'STEP_FINISHED' for step \"{}\" attributed to {}: that step is open under {}. A step must be finished by whoever started it.",
                            e.step_name,
                            describe_owner(owner),
                            describe_owner(other)
                        ),
                        None => format!(
                            "Cannot send 'STEP_FINISHED' for step \"{}\" that was not started",
                            e.step_name
                        ),
                    }));
                }
            }
            Event::ActivitySnapshot(e) => {
                // Only a replacing snapshot re-owns an existing activity.
                let is_new = !self.activity_owners.contains_key(&e.message_id);
                if is_new || e.replace != Some(false) {
                    self.activity_owners
                        .insert(e.message_id.clone(), e.subagent_run_id.clone());
                }
            }
            Event::ActivityDelta(e) => {
                if let Some(owner) = self.activity_owners.get(&e.message_id) {
                    check_owner(event_type, tag, owner, "activity", &e.message_id)?;
                }
            }
            Event::ReasoningStart(e) => {
                self.open_reasoning(event_type, tag, &e.message_id, true)?;
            }
            Event::ReasoningMessageStart(e) => {
                self.open_reasoning(event_type, tag, &e.message_id, false)?;
            }
            Event::ReasoningMessageContent(e) => {
                self.continue_reasoning(event_type, tag, &e.message_id, false, false)?;
            }
            Event::ReasoningMessageEnd(e) => {
                self.continue_reasoning(event_type, tag, &e.message_id, false, true)?;
            }
            Event::ReasoningEnd(e) => {
                self.continue_reasoning(event_type, tag, &e.message_id, true, true)?;
            }
            Event::ReasoningEncryptedValue(e) => {
                let id = &e.entity_id;
                let (owner, kind) = match e.subtype {
                    ReasoningEncryptedValueSubtype::ToolCall => {
                        (self.tool_call_owners.get(id), "tool call")
                    }
                    ReasoningEncryptedValueSubtype::Message => (
                        self.message_owners
                            .get(id)
                            .or_else(|| self.reasoning_owners.get(id)),
                        "message",
                    ),
                };
                // An unknown entity is tolerated: the consumer may drop it.
                if let Some(owner) = owner {
                    check_owner(event_type, tag, owner, kind, id)?;
                }
            }
            Event::SubagentStarted(e) => {
                let id = &e.subagent_run_id;
                if self.active_subagents.contains(id) {
                    return Err(ProtocolError::new(format!(
                        "Cannot send 'SUBAGENT_STARTED': subagent '{id}' is already active. Finish it with 'SUBAGENT_FINISHED' first."
                    )));
                }
                if self.closed_subagents.contains(id) {
                    return Err(ProtocolError::new(format!(
                        "Cannot send 'SUBAGENT_STARTED': subagent '{id}' has already finished in this run. Subagent IDs are per-invocation and cannot be reused."
                    )));
                }
                if let Some(parent) = &e.parent_subagent_run_id
                    && !self.active_subagents.contains(parent)
                    && !self.closed_subagents.contains(parent)
                {
                    return Err(ProtocolError::new(format!(
                        "Cannot send 'SUBAGENT_STARTED': parentSubagentRunId '{parent}' has not been started in this run."
                    )));
                }
                self.active_subagents.push(id.clone());
            }
            Event::SubagentFinished(e) => self.close_subagent(event_type, &e.subagent_run_id)?,
            Event::SubagentError(e) => self.close_subagent(event_type, &e.subagent_run_id)?,
            Event::MessagesSnapshot(e) => self.seed_owners(&e.messages, true),
            Event::RunStarted(e) => {
                self.run_started = true;
                if let Some(input) = &e.input {
                    self.seed_owners(&input.messages, false);
                }
            }
            Event::RunFinished(_) => {
                self.check_all_closed()?;
                self.run_finished = true;
            }
            Event::RunError(_) => self.run_errored = true,
            _ => {}
        }
        Ok(())
    }

    fn close_subagent(&mut self, event_type: &str, id: &str) -> Result<(), ProtocolError> {
        let Some(index) = self.active_subagents.iter().position(|s| s == id) else {
            return Err(ProtocolError::new(format!(
                "Cannot send '{event_type}': no active subagent found with ID '{id}'. A 'SUBAGENT_STARTED' event must be sent first."
            )));
        };
        self.active_subagents.remove(index);
        self.closed_subagents.insert(id.to_owned());
        Ok(())
    }

    fn continue_message(
        &self,
        event_type: &str,
        tag: Option<&str>,
        id: &str,
        hint: &str,
    ) -> Result<(), ProtocolError> {
        if !self.active_messages.contains(id) {
            return Err(ProtocolError::new(format!(
                "Cannot send '{event_type}' event: No active text message found with ID '{id}'. {hint}"
            )));
        }
        match self.message_owners.get(id) {
            Some(owner) => check_owner(event_type, tag, owner, "message", id),
            None => Ok(()),
        }
    }

    fn continue_tool_call(
        &self,
        event_type: &str,
        tag: Option<&str>,
        id: &str,
        hint: &str,
    ) -> Result<(), ProtocolError> {
        if !self.active_tool_calls.contains(id) {
            return Err(ProtocolError::new(format!(
                "Cannot send '{event_type}' event: No active tool call found with ID '{id}'. {hint}"
            )));
        }
        match self.tool_call_owners.get(id) {
            Some(owner) => check_owner(event_type, tag, owner, "tool call", id),
            None => Ok(()),
        }
    }

    // Spans (REASONING_START/END) and reasoning messages are separate
    // namespaces: one id may name both. Ownership is shared across them.
    fn open_reasoning(
        &mut self,
        event_type: &str,
        tag: Option<&str>,
        id: &str,
        span: bool,
    ) -> Result<(), ProtocolError> {
        let open = if span {
            &self.active_reasoning_spans
        } else {
            &self.active_reasoning_messages
        };
        if open.contains(id) {
            return Err(ProtocolError::new(if span {
                format!(
                    "Cannot send 'REASONING_START' event: A reasoning span with ID '{id}' is already in progress. Complete it with 'REASONING_END' first."
                )
            } else {
                format!(
                    "Cannot send 'REASONING_MESSAGE_START' event: A reasoning message with ID '{id}' is already in progress. Complete it with 'REASONING_MESSAGE_END' first."
                )
            }));
        }
        match self.reasoning_owners.get(id) {
            Some(owner) => check_owner(event_type, tag, owner, "reasoning message", id)?,
            None => {
                self.reasoning_owners
                    .insert(id.to_owned(), tag.map(str::to_owned));
            }
        }
        if span {
            self.active_reasoning_spans.insert(id.to_owned());
        } else {
            self.active_reasoning_messages.insert(id.to_owned());
        }
        Ok(())
    }

    fn continue_reasoning(
        &mut self,
        event_type: &str,
        tag: Option<&str>,
        id: &str,
        span: bool,
        closes: bool,
    ) -> Result<(), ProtocolError> {
        let open = if span {
            &mut self.active_reasoning_spans
        } else {
            &mut self.active_reasoning_messages
        };
        if !open.contains(id) {
            return Err(ProtocolError::new(if span {
                format!(
                    "Cannot send 'REASONING_END' event: No active reasoning span found with ID '{id}'. A 'REASONING_START' event must be sent first."
                )
            } else {
                format!(
                    "Cannot send '{event_type}' event: No active reasoning message found with ID '{id}'. Start a reasoning message with 'REASONING_MESSAGE_START' first."
                )
            }));
        }
        if closes {
            open.remove(id);
        }
        match self.reasoning_owners.get(id) {
            Some(owner) => check_owner(event_type, tag, owner, "reasoning message", id),
            None => Ok(()),
        }
    }

    fn check_all_closed(&self) -> Result<(), ProtocolError> {
        let open_steps: Vec<String> = self
            .active_steps
            .iter()
            .flat_map(|(owner, steps)| {
                steps.iter().map(move |name| match owner {
                    Some(owner) => format!("{name} (subagent '{owner}')"),
                    None => name.clone(),
                })
            })
            .collect();
        if !open_steps.is_empty() {
            return Err(still_active("steps", open_steps));
        }
        let sets = [
            ("text messages", &self.active_messages),
            ("reasoning messages", &self.active_reasoning_messages),
            ("reasoning spans", &self.active_reasoning_spans),
            ("tool calls", &self.active_tool_calls),
        ];
        for (what, set) in sets {
            if !set.is_empty() {
                let mut ids: Vec<String> = set.iter().cloned().collect();
                ids.sort();
                return Err(still_active(what, ids));
            }
        }
        if !self.active_subagents.is_empty() {
            return Err(still_active("subagents", self.active_subagents.clone()));
        }
        Ok(())
    }
}

fn still_active(what: &str, ids: Vec<String>) -> ProtocolError {
    ProtocolError::new(format!(
        "Cannot send 'RUN_FINISHED' while {what} are still active: {}",
        ids.join(", ")
    ))
}

fn describe_owner(owner: &Owner) -> String {
    match owner {
        Some(owner) => format!("subagent '{owner}'"),
        None => "the parent agent".to_owned(),
    }
}

/// A continuation's tag must agree with its entity's owner. An absent tag
/// always agrees; an untagged opener belongs to the parent agent.
fn check_owner(
    event_type: &str,
    tag: Option<&str>,
    owner: &Owner,
    entity: &str,
    id: &str,
) -> Result<(), ProtocolError> {
    match tag {
        Some(tag) if Some(tag) != owner.as_deref() => Err(ProtocolError::new(format!(
            "Cannot send '{event_type}': subagentRunId '{tag}' does not match the {entity} '{id}' opener's subagent '{}'.",
            owner_label(owner)
        ))),
        _ => Ok(()),
    }
}

/// The opener synthesised from a chunk carries the chunk's metadata but not
/// its provider payload; that rides the content event.
fn chunk_opener_base(base: &BaseEvent) -> BaseEvent {
    BaseEvent {
        timestamp: base.timestamp,
        raw_event: None,
        metadata: base.metadata.clone(),
    }
}

/// A chunk becomes a content event when it carries a delta or a provider
/// payload, or when it carries only metadata and nothing else was emitted
/// for it (so the metadata still reaches the message).
fn emits_content(has_delta: bool, base: &BaseEvent, nothing_emitted: bool) -> bool {
    has_delta || base.raw_event.is_some() || (base.metadata.is_some() && nothing_emitted)
}

fn role_str(role: TextMessageRole) -> &'static str {
    match role {
        TextMessageRole::Developer => "developer",
        TextMessageRole::System => "system",
        TextMessageRole::Assistant => "assistant",
        TextMessageRole::User => "user",
    }
}

/// The optional `subagentRunId` attribution tag. Lifecycle events carry the
/// subagent's identity instead, which is not attribution.
pub(crate) fn attribution(event: &Event) -> Option<&str> {
    let tag = match event {
        Event::TextMessageStart(e) => &e.subagent_run_id,
        Event::TextMessageContent(e) => &e.subagent_run_id,
        Event::TextMessageEnd(e) => &e.subagent_run_id,
        Event::TextMessageChunk(e) => &e.subagent_run_id,
        Event::ToolCallStart(e) => &e.subagent_run_id,
        Event::ToolCallArgs(e) => &e.subagent_run_id,
        Event::ToolCallEnd(e) => &e.subagent_run_id,
        Event::ToolCallChunk(e) => &e.subagent_run_id,
        Event::ToolCallResult(e) => &e.subagent_run_id,
        Event::StateSnapshot(e) => &e.subagent_run_id,
        Event::StateDelta(e) => &e.subagent_run_id,
        Event::ActivitySnapshot(e) => &e.subagent_run_id,
        Event::ActivityDelta(e) => &e.subagent_run_id,
        Event::Raw(e) => &e.subagent_run_id,
        Event::Custom(e) => &e.subagent_run_id,
        Event::StepStarted(e) | Event::StepFinished(e) => &e.subagent_run_id,
        Event::ReasoningStart(e) | Event::ReasoningEnd(e) => &e.subagent_run_id,
        Event::ReasoningMessageStart(e) => &e.subagent_run_id,
        Event::ReasoningMessageContent(e) => &e.subagent_run_id,
        Event::ReasoningMessageEnd(e) => &e.subagent_run_id,
        Event::ReasoningMessageChunk(e) => &e.subagent_run_id,
        Event::ReasoningEncryptedValue(e) => &e.subagent_run_id,
        Event::MessagesSnapshot(_)
        | Event::RunStarted(_)
        | Event::RunFinished(_)
        | Event::RunError(_)
        | Event::SubagentStarted(_)
        | Event::SubagentFinished(_)
        | Event::SubagentError(_) => return None,
    };
    tag.as_deref()
}
