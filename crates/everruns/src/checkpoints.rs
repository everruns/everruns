//! Conversation checkpoints over the branch-aware journal (EVE-1339).
//!
//! Decision: a checkpoint is a named point between turns, never a copy of
//! history. It retains the active branch's journal ranges through the end of
//! its last whole turn. Restoring selects those ranges as a new conversation
//! branch through the existing branch-selection command (EVE-1305), so the
//! journal is never rewritten and raw export keeps every event:
//!
//! - [`RestoreMode::Branch`] retains the undone tail as a branch that
//!   [`Checkpoints::redo`] reselects.
//! - [`RestoreMode::Truncate`] drops the undone tail from the branch catalog,
//!   so it can no longer be reselected; checkpoints taken on it become
//!   unreachable and are pruned.
//!
//! Checkpoints live in the session authority next to its branches, keyed by
//! session. Every lookup is scoped to the session the handle was opened for,
//! and a checkpoint only ever names ranges of that session's own journal.
//! Only branch-capable authorities (`SqliteHosting`) support them; the
//! volatile default refuses.
// THREAT[TM-DURABLE-015]: session-scoped records, journal-validated ranges,
// non-destructive restore, bounded storage and display-safe previews.
use crate::Engine;
use crate::hosting::spi::events::{Event, EventData};
use crate::hosting::spi::session_commands::SessionCommand;
use crate::hosting::spi::{EventReadLimit, EventReadRequest};
use crate::hosting::{HistoryLineage, HistorySelection, SequenceRange, SessionHosting};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Retained checkpoints per session; delete old ones to make room.
pub const MAX_CONVERSATION_CHECKPOINTS: usize = 1024;
/// Turns listed per side of a [`RestorePreview`]; totals are always exact.
pub const MAX_PREVIEW_TURNS: usize = 50;
/// Characters kept from a label or message in display previews.
const PREVIEW_CHARS: usize = 160;
/// Journal events read past a checkpoint to pick up late events of its last
/// turn (`session.idled` follows the terminal).
const SETTLE_WINDOW: usize = 64;

/// A retained checkpoint as the session authority stores it.
///
/// Hosts read it through [`CheckpointSummary`]; only hosting plugins
/// implementing the checkpoint SPI handle it directly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationCheckpoint {
    /// Session-scoped identity.
    pub id: uuid::Uuid,
    /// Caller label, display-safe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Branch that was active when the checkpoint was taken.
    pub branch: String,
    /// Last journal sequence the checkpoint covers (`0` before any event).
    pub through: i32,
    /// Journal ranges the conversation holds at this point.
    pub ranges: Vec<SequenceRange>,
    /// Input sequences at or before `through` that belong to later turns,
    /// so a queued message never re-enters context without its turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded: Vec<i32>,
    /// Number of turns in the conversation at this point.
    pub turn_index: u32,
    /// Last turn included, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    /// Input message that started the last included turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<MessageId>,
    /// Display-safe excerpt of that input message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// RFC 3339 UTC creation time.
    pub created_at: String,
}

/// A reachable checkpoint as a host lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointSummary {
    /// The retained checkpoint.
    #[serde(flatten)]
    pub checkpoint: ConversationCheckpoint,
    /// The checkpoint is a point on the active branch, so restoring it only
    /// drops later turns. Otherwise it lies on a retained branch and restoring
    /// it also brings that branch's earlier turns back.
    pub on_active_branch: bool,
}

impl std::ops::Deref for CheckpointSummary {
    type Target = ConversationCheckpoint;
    fn deref(&self) -> &ConversationCheckpoint {
        &self.checkpoint
    }
}

/// How a restore treats the turns it removes from context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreMode {
    /// Retain the undone turns as a branch [`Checkpoints::redo`] can reselect.
    Branch,
    /// Drop the undone turns from the branch catalog. They stay in the raw
    /// journal for audit and export but can no longer be reselected.
    Truncate,
}

/// One turn a restore would remove or bring back, display-safe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnPreview {
    /// Turn identity.
    pub turn_id: TurnId,
    /// Input message that started the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<MessageId>,
    /// Excerpt of that input message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// Excerpt of the turn's last agent message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}

/// What restoring a checkpoint would change, read without changing anything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestorePreview {
    /// The checkpoint to restore.
    pub checkpoint: CheckpointSummary,
    /// Branch that is active now; [`RestoreMode::Branch`] retains it as the
    /// redo target.
    pub active_branch: String,
    /// Lineage revision the preview read.
    pub revision: u64,
    /// `false` when the conversation already is at the checkpoint.
    pub changes: bool,
    /// Turns that leave context, oldest first, at most [`MAX_PREVIEW_TURNS`].
    pub dropped: Vec<TurnPreview>,
    /// Exact number of turns that leave context.
    pub dropped_total: usize,
    /// Turns from a retained branch that re-enter context, oldest first.
    pub restored: Vec<TurnPreview>,
    /// Exact number of turns that re-enter context.
    pub restored_total: usize,
}

/// Result of a restore or redo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreOutcome {
    /// Lineage revision after the change.
    pub revision: u64,
    /// Branch now active.
    pub branch: String,
    /// `false` when nothing changed.
    pub changed: bool,
    /// Retained branch holding the undone turns, for [`Checkpoints::redo`].
    /// `None` after a truncating restore or when nothing changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undone: Option<String>,
}

impl Engine {
    /// Conversation checkpoints of one session (EVE-1339): create, list,
    /// preview, restore and redo against the session authority. Requires a
    /// branch-capable hosting profile such as `SqliteHosting`.
    /// Privileged hosts authorize callers. Stability: experimental.
    pub fn checkpoints(&self, session: SessionId) -> Checkpoints {
        Checkpoints {
            engine: self.clone(),
            session,
        }
    }
}

/// Checkpoint handle for one session, from [`Engine::checkpoints`].
///
/// Creating, listing and previewing work at any time, including from a tool
/// while a turn runs: a checkpoint then marks the end of the last finished
/// turn. Restoring and redoing change the active branch, which the authority
/// accepts only while the session is quiescent (no running turn, waits or
/// unconsumed input); otherwise they fail without changing anything.
#[derive(Clone)]
pub struct Checkpoints {
    engine: Engine,
    session: SessionId,
}

impl Checkpoints {
    /// The session this handle is scoped to.
    pub fn session_id(&self) -> SessionId {
        self.session
    }

    /// Mark the end of the last finished turn on the active branch.
    pub async fn create(&self) -> Result<CheckpointSummary> {
        self.create_checkpoint(None).await
    }

    /// [`create`](Self::create) with a label for display.
    pub async fn create_labeled(&self, label: impl AsRef<str>) -> Result<CheckpointSummary> {
        let label = display(label.as_ref());
        self.create_checkpoint((!label.is_empty()).then_some(label))
            .await
    }

    async fn create_checkpoint(&self, label: Option<String>) -> Result<CheckpointSummary> {
        let profile = self.profile()?;
        let lineage = profile.history_lineage(self.session).await?;
        let fold = self.fold_active(&profile).await?;
        let active = lineage.active_ranges();
        let boundary = fold.boundary(active);
        // An admitted input no included turn holds belongs to a later turn
        // (queued or not yet run): it never re-enters context without it.
        let mut excluded = Vec::new();
        for (message, sequence) in fold.unheld_inputs(boundary.0) {
            if profile.receipt(self.session, message).await?.is_some() {
                excluded.push(sequence);
            }
        }
        let checkpoint = fold.checkpoint(active, boundary, excluded, label, &lineage.active);
        // Unreachable checkpoints do not count against capacity.
        self.prune(&profile, &lineage).await?;
        profile
            .save_conversation_checkpoint(self.session, checkpoint.clone())
            .await?;
        Ok(CheckpointSummary {
            on_active_branch: on_branch(active, &checkpoint),
            checkpoint,
        })
    }

    /// Reachable checkpoints, oldest first. A checkpoint taken on turns a
    /// truncating restore dropped is no longer listed.
    pub async fn list(&self) -> Result<Vec<CheckpointSummary>> {
        let profile = self.profile()?;
        let lineage = profile.history_lineage(self.session).await?;
        let mut listed: Vec<CheckpointSummary> = profile
            .conversation_checkpoints(self.session)
            .await?
            .into_iter()
            .filter(|checkpoint| reachable(&lineage, checkpoint).is_some())
            .map(|checkpoint| CheckpointSummary {
                on_active_branch: on_branch(lineage.active_ranges(), &checkpoint),
                checkpoint,
            })
            .collect();
        listed.sort_by(|a, b| (&a.created_at, a.id).cmp(&(&b.created_at, b.id)));
        Ok(listed)
    }

    /// One reachable checkpoint.
    pub async fn get(&self, checkpoint: uuid::Uuid) -> Result<CheckpointSummary> {
        self.list()
            .await?
            .into_iter()
            .find(|summary| summary.id == checkpoint)
            .ok_or_else(not_found)
    }

    /// Forget a checkpoint. The conversation and its branches are unchanged.
    pub async fn delete(&self, checkpoint: uuid::Uuid) -> Result<bool> {
        self.profile()?
            .delete_conversation_checkpoint(self.session, checkpoint)
            .await
    }

    /// Describe what [`restore`](Self::restore) would change, without
    /// changing anything.
    pub async fn preview(&self, checkpoint: uuid::Uuid) -> Result<RestorePreview> {
        let profile = self.profile()?;
        let (lineage, summary, target) = self.resolve(&profile, checkpoint).await?;
        let active = lineage.active_ranges().to_vec();
        let current = self.fold_active(&profile).await?;
        let dropped: Vec<TurnPreview> = current
            .turns
            .iter()
            .filter(|turn| !contains(&target, turn.first))
            .map(|turn| current.preview(turn))
            .collect();
        let restored: Vec<TurnPreview> = if subset(&target, &active) {
            Vec::new()
        } else {
            let mut fold = Fold::default();
            self.read_ranges(&profile, &target, |event| fold.add(event))
                .await?;
            fold.turns
                .iter()
                .filter(|turn| !contains(&active, turn.first))
                .map(|turn| fold.preview(turn))
                .collect()
        };
        Ok(RestorePreview {
            changes: !same(&target, &active),
            active_branch: lineage.active.clone(),
            revision: lineage.revision,
            dropped_total: dropped.len(),
            restored_total: restored.len(),
            dropped: dropped.into_iter().take(MAX_PREVIEW_TURNS).collect(),
            restored: restored.into_iter().take(MAX_PREVIEW_TURNS).collect(),
            checkpoint: summary,
        })
    }

    /// Return the conversation to `checkpoint`. Context, history and new
    /// input follow the restored branch; the journal is never rewritten.
    pub async fn restore(
        &self,
        checkpoint: uuid::Uuid,
        mode: RestoreMode,
    ) -> Result<RestoreOutcome> {
        let profile = self.profile()?;
        let (lineage, _, target) = self.resolve(&profile, checkpoint).await?;
        if same(&target, lineage.active_ranges()) {
            return Ok(unchanged(&lineage));
        }
        // Revisions only grow, so this identity was never selected before.
        let branch = format!("restore-{}", lineage.revision + 1);
        let selection: HistorySelection = serde_json::from_value(
            profile
                .command(
                    self.session,
                    SessionCommand::SelectBranch {
                        id: uuid::Uuid::new_v4().to_string(),
                        expected_revision: lineage.revision,
                        branch,
                        ranges: Some(target),
                        discard: mode == RestoreMode::Truncate,
                    },
                )
                .await?,
        )
        .map_err(|e| AgentLoopError::store(e.to_string()))?;
        if mode == RestoreMode::Truncate {
            let lineage = profile.history_lineage(self.session).await?;
            self.prune(&profile, &lineage).await?;
        }
        Ok(RestoreOutcome {
            undone: (mode == RestoreMode::Branch && selection.changed)
                .then(|| lineage.active.clone()),
            revision: selection.revision,
            branch: selection.branch,
            changed: selection.changed,
        })
    }

    /// Reselect a retained branch, such as [`RestoreOutcome::undone`]. The
    /// branch left behind is retained in turn, so a redo can be undone.
    pub async fn redo(&self, branch: &str) -> Result<RestoreOutcome> {
        let profile = self.profile()?;
        let lineage = profile.history_lineage(self.session).await?;
        if branch == lineage.active {
            return Ok(unchanged(&lineage));
        }
        if lineage.branch(branch).is_none() {
            return Err(AgentLoopError::config("history branch not found"));
        }
        let selection = profile
            .select_history_branch(
                self.session,
                uuid::Uuid::new_v4(),
                lineage.revision,
                branch,
                None,
            )
            .await?;
        Ok(RestoreOutcome {
            undone: selection.changed.then(|| lineage.active.clone()),
            revision: selection.revision,
            branch: selection.branch,
            changed: selection.changed,
        })
    }

    fn profile(&self) -> Result<Arc<dyn SessionHosting>> {
        self.engine.profile().cloned()
    }

    /// The checkpoint, the lineage it was resolved against and the ranges a
    /// restore selects: its own ranges plus late events of its last turn.
    async fn resolve(
        &self,
        profile: &Arc<dyn SessionHosting>,
        id: uuid::Uuid,
    ) -> Result<(HistoryLineage, CheckpointSummary, Vec<SequenceRange>)> {
        let lineage = profile.history_lineage(self.session).await?;
        let checkpoint = profile
            .conversation_checkpoints(self.session)
            .await?
            .into_iter()
            .find(|checkpoint| checkpoint.id == id)
            .ok_or_else(not_found)?;
        let branch = reachable(&lineage, &checkpoint)
            .ok_or_else(|| {
                AgentLoopError::config("conversation checkpoint is no longer reachable")
            })?
            .to_vec();
        let mut target = checkpoint.ranges.clone();
        if let Some(turn) = checkpoint.turn_id {
            let after = profile
                .history_events(
                    self.session,
                    branch.clone(),
                    checkpoint.through,
                    SETTLE_WINDOW,
                )
                .await?;
            for event in after {
                if event.context.turn_id == Some(turn)
                    && let Some(sequence) = event.sequence
                {
                    target.push(SequenceRange::new(sequence, sequence));
                }
            }
            target = normalize(target);
        }
        let summary = CheckpointSummary {
            on_active_branch: on_branch(lineage.active_ranges(), &checkpoint),
            checkpoint,
        };
        Ok((lineage, summary, target))
    }

    /// Drop checkpoints no retained branch reaches any more.
    async fn prune(
        &self,
        profile: &Arc<dyn SessionHosting>,
        lineage: &HistoryLineage,
    ) -> Result<()> {
        for checkpoint in profile.conversation_checkpoints(self.session).await? {
            if reachable(lineage, &checkpoint).is_none() {
                profile
                    .delete_conversation_checkpoint(self.session, checkpoint.id)
                    .await?;
            }
        }
        Ok(())
    }

    /// Fold the active branch through the authority's read-only reader.
    async fn fold_active(&self, profile: &Arc<dyn SessionHosting>) -> Result<Fold> {
        let reader = profile.reader(self.session).event_log;
        let limit = EventReadLimit::new(512).map_err(|e| AgentLoopError::store(e.to_string()))?;
        let mut request = EventReadRequest::new(self.session, limit);
        let mut fold = Fold::default();
        loop {
            let page = reader
                .read_page(request)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()))?;
            for event in &page.events {
                fold.add(event);
            }
            match page.next_cursor {
                Some(cursor) => {
                    request = EventReadRequest::new(self.session, limit).with_cursor(cursor)
                }
                None => return Ok(fold),
            }
        }
    }

    /// Visit every journal event in `ranges`, active or not.
    async fn read_ranges(
        &self,
        profile: &Arc<dyn SessionHosting>,
        ranges: &[SequenceRange],
        mut visit: impl FnMut(&Event),
    ) -> Result<()> {
        const PAGE: usize = 256;
        let mut after = 0;
        loop {
            let events = profile
                .history_events(self.session, ranges.to_vec(), after, PAGE)
                .await?;
            for event in &events {
                visit(event);
            }
            match events.last().and_then(|event| event.sequence) {
                Some(last) if events.len() == PAGE => after = last,
                _ => return Ok(()),
            }
        }
    }
}

fn not_found() -> AgentLoopError {
    AgentLoopError::config("conversation checkpoint not found")
}

fn unchanged(lineage: &HistoryLineage) -> RestoreOutcome {
    RestoreOutcome {
        revision: lineage.revision,
        branch: lineage.active.clone(),
        changed: false,
        undone: None,
    }
}

/// One turn as the fold sees it.
struct TurnInfo {
    id: TurnId,
    first: i32,
    last: i32,
    ended: bool,
    message: Option<MessageId>,
    reply: Option<String>,
}

/// Turn structure of a sequence of journal events, in sequence order.
#[derive(Default)]
struct Fold {
    turns: Vec<TurnInfo>,
    index: HashMap<TurnId, usize>,
    /// Input message events: sequence and excerpt.
    inputs: HashMap<MessageId, (i32, Option<String>)>,
}

impl Fold {
    fn add(&mut self, event: &Event) {
        let Some(sequence) = event.sequence else {
            return;
        };
        if let EventData::InputMessage(data) = &event.data {
            let text = data.message.text().map(display);
            self.inputs.insert(data.message.id, (sequence, text));
        }
        let Some(turn) = event.context.turn_id else {
            return;
        };
        let index = *self.index.entry(turn).or_insert_with(|| {
            self.turns.push(TurnInfo {
                id: turn,
                first: sequence,
                last: sequence,
                ended: false,
                message: None,
                reply: None,
            });
            self.turns.len() - 1
        });
        let info = &mut self.turns[index];
        info.last = info.last.max(sequence);
        info.message = info.message.or(event.context.input_message_id);
        info.ended |= matches!(
            event.data,
            EventData::TurnCompleted(_)
                | EventData::TurnCancelled(_)
                | EventData::TurnFailed(_)
                | EventData::TurnSealed(_)
        );
        if let EventData::OutputMessageCompleted(data) = &event.data
            && let Some(text) = data.message.text()
        {
            info.reply = Some(display(text));
        }
    }

    fn input(&self, turn: &TurnInfo) -> Option<&(i32, Option<String>)> {
        turn.message.and_then(|message| self.inputs.get(&message))
    }

    fn preview(&self, turn: &TurnInfo) -> TurnPreview {
        TurnPreview {
            turn_id: turn.id,
            message_id: turn.message,
            input: self.input(turn).and_then(|(_, text)| text.clone()),
            reply: turn.reply.clone(),
        }
    }

    /// The end of the last turn that finished before any unfinished one
    /// began, and that turn. Before the first turn, the position just before
    /// it, so seeded history stays.
    fn boundary(&self, active: &[SequenceRange]) -> (i32, Option<&TurnInfo>) {
        let open = self
            .turns
            .iter()
            .filter(|turn| !turn.ended)
            .map(|turn| turn.first)
            .min()
            .unwrap_or(i32::MAX);
        let last = self
            .turns
            .iter()
            .filter(|turn| turn.ended && turn.last < open)
            .max_by_key(|turn| turn.last);
        let through = match last {
            Some(turn) => turn.last,
            // Before the first turn: keep seeded history, not its input.
            None => self
                .turns
                .iter()
                .map(|turn| {
                    self.input(turn)
                        .map_or(turn.first, |(input, _)| (*input).min(turn.first))
                })
                .min()
                .map_or_else(
                    || active.last().map_or(0, |range| range.last),
                    |start| start - 1,
                ),
        };
        (through, last)
    }

    /// Input messages at or before `through` that no included turn holds,
    /// either as its input or steered into its span. Admitted ones among
    /// them belong to later turns; seeded history has no admission.
    fn unheld_inputs(&self, through: i32) -> Vec<(MessageId, i32)> {
        let included: Vec<&TurnInfo> = self
            .turns
            .iter()
            .filter(|turn| turn.last <= through)
            .collect();
        let mut unheld: Vec<(MessageId, i32)> = self
            .inputs
            .iter()
            .filter(|(id, (sequence, _))| {
                *sequence <= through
                    && !included.iter().any(|turn| {
                        turn.message == Some(**id)
                            || (turn.first <= *sequence && *sequence <= turn.last)
                    })
            })
            .map(|(id, (sequence, _))| (*id, *sequence))
            .collect();
        unheld.sort_unstable_by_key(|(_, sequence)| *sequence);
        unheld
    }

    /// A checkpoint at `through`, after `last`, leaving out `excluded`.
    fn checkpoint(
        &self,
        active: &[SequenceRange],
        (through, last): (i32, Option<&TurnInfo>),
        mut excluded: Vec<i32>,
        label: Option<String>,
        branch: &str,
    ) -> ConversationCheckpoint {
        excluded.sort_unstable();
        let ranges = subtract(&prefix(active, through), &excluded);
        ConversationCheckpoint {
            id: uuid::Uuid::now_v7(),
            label,
            branch: branch.to_owned(),
            through,
            ranges,
            excluded,
            turn_index: self
                .turns
                .iter()
                .filter(|turn| turn.last <= through)
                .count() as u32,
            turn_id: last.map(|turn| turn.id),
            message_id: last.and_then(|turn| turn.message),
            preview: last.and_then(|turn| self.input(turn).and_then(|(_, text)| text.clone())),
            created_at: everruns_contracts::rt::utc_now().to_rfc3339(),
        }
    }
}

/// The retained branch a checkpoint lies on, the active one preferred.
fn reachable<'a>(
    lineage: &'a HistoryLineage,
    checkpoint: &ConversationCheckpoint,
) -> Option<&'a [SequenceRange]> {
    let active = lineage.active_ranges();
    if on_branch(active, checkpoint) {
        return Some(active);
    }
    lineage
        .branches
        .iter()
        .map(|branch| branch.ranges.as_slice())
        .find(|ranges| on_branch(ranges, checkpoint))
}

/// The branch holds exactly the checkpoint's ranges up to its sequence.
fn on_branch(branch: &[SequenceRange], checkpoint: &ConversationCheckpoint) -> bool {
    same(
        &subtract(&prefix(branch, checkpoint.through), &checkpoint.excluded),
        &checkpoint.ranges,
    )
}

/// `ranges` up to and including `through`.
fn prefix(ranges: &[SequenceRange], through: i32) -> Vec<SequenceRange> {
    ranges
        .iter()
        .filter(|range| range.first <= through)
        .map(|range| SequenceRange::new(range.first, range.last.min(through)))
        .collect()
}

/// `ranges` without the single sequences in `holes`.
fn subtract(ranges: &[SequenceRange], holes: &[i32]) -> Vec<SequenceRange> {
    let mut out = Vec::new();
    for range in ranges {
        let mut first = range.first;
        for hole in holes
            .iter()
            .filter(|hole| range.first <= **hole && **hole <= range.last)
        {
            if first < *hole {
                out.push(SequenceRange::new(first, hole - 1));
            }
            first = hole + 1;
        }
        if first <= range.last {
            out.push(SequenceRange::new(first, range.last));
        }
    }
    out
}

/// Sorted, with touching or overlapping ranges merged: the journal assigns
/// dense sequences, so equal sets of events compare equal.
fn normalize(mut ranges: Vec<SequenceRange>) -> Vec<SequenceRange> {
    ranges.sort_by_key(|range| (range.first, range.last));
    let mut out: Vec<SequenceRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match out.last_mut() {
            Some(last) if range.first <= last.last.saturating_add(1) => {
                last.last = last.last.max(range.last)
            }
            _ => out.push(range),
        }
    }
    out
}

fn same(a: &[SequenceRange], b: &[SequenceRange]) -> bool {
    normalize(a.to_vec()) == normalize(b.to_vec())
}

fn contains(ranges: &[SequenceRange], sequence: i32) -> bool {
    ranges
        .iter()
        .any(|range| range.first <= sequence && sequence <= range.last)
}

/// Every sequence of `inner` lies in `outer`.
fn subset(inner: &[SequenceRange], outer: &[SequenceRange]) -> bool {
    let outer = normalize(outer.to_vec());
    inner.iter().all(|range| {
        outer
            .iter()
            .any(|o| o.first <= range.first && range.last <= o.last)
    })
}

/// Display-safe excerpt: control and bidirectional formatting characters
/// become spaces, whitespace collapses and long text is cut at a character
/// boundary, so model or user text cannot reshape a host's UI line.
fn display(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c.is_control()
                || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut out = String::new();
    for (count, word) in cleaned.split_whitespace().enumerate() {
        if count > 0 {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.chars().count() > PREVIEW_CHARS {
        out = out.chars().take(PREVIEW_CHARS - 1).collect();
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(first: i32, last: i32) -> SequenceRange {
        SequenceRange::new(first, last)
    }

    #[test]
    fn range_helpers_compare_event_sets() {
        assert_eq!(prefix(&[r(1, 3), r(7, 9)], 8), [r(1, 3), r(7, 8)]);
        assert_eq!(prefix(&[r(1, 3), r(7, 9)], 5), [r(1, 3)]);
        assert_eq!(subtract(&[r(1, 9)], &[1, 4, 9]), [r(2, 3), r(5, 8)]);
        assert!(same(&[r(1, 3), r(4, 6)], &[r(1, 6)]));
        assert!(!same(&[r(1, 3)], &[r(1, 4)]));
        assert!(subset(&[r(2, 3), r(5, 5)], &[r(1, 3), r(4, 6)]));
        assert!(!subset(&[r(2, 7)], &[r(1, 6)]));
    }

    #[test]
    fn checkpoints_hold_only_their_branch_prefix() {
        let checkpoint = ConversationCheckpoint {
            id: uuid::Uuid::nil(),
            label: None,
            branch: "main".into(),
            through: 6,
            ranges: vec![r(1, 3), r(5, 6)],
            excluded: vec![4],
            turn_index: 1,
            turn_id: None,
            message_id: None,
            preview: None,
            created_at: String::new(),
        };
        assert!(on_branch(&[r(1, 9)], &checkpoint), "queued input excluded");
        assert!(on_branch(&[r(1, 6)], &checkpoint));
        assert!(!on_branch(&[r(1, 5)], &checkpoint), "tail missing");
        assert!(!on_branch(&[r(1, 3), r(7, 9)], &checkpoint));
    }

    #[test]
    fn display_neutralizes_control_and_bidi_characters() {
        assert_eq!(display("a\nb\t\u{202e}c\u{1b}[2J"), "a b c [2J");
        let long = "x".repeat(400);
        let shown = display(&long);
        assert_eq!(shown.chars().count(), PREVIEW_CHARS);
        assert!(shown.ends_with('…'));
    }
}
