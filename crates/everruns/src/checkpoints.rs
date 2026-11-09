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
//! - [`RestoreMode::Truncate`] drops the current branch from the branch
//!   catalog, so it can no longer be reselected; checkpoints only it held
//!   become unreachable.
//! - [`Checkpoints::delete_branch`] removes a retained branch the same way,
//!   keeping the catalog under its limit.
//!
//! Unreachable checkpoints are hidden from every read and pruned by the
//! authority inside the transaction that commits the next checkpoint. They
//! are never pruned client side: `delete_branch` needs no quiescence, so a
//! lineage read, a list and a delete in separate transactions would race a
//! tool creating a checkpoint on newer turns and delete it.
//!
//! The authority builds each checkpoint in one transaction from its own
//! snapshot of the active branch and input assignments (see
//! [`CheckpointBuilder`]), so a concurrent append or branch change cannot
//! leave a checkpoint off its branch, and an input belongs to a checkpoint
//! only when it is assigned to an included turn.
//!
//! Checkpoints live in the session authority next to its branches, keyed by
//! session. Every lookup is scoped to the session the handle was opened for,
//! and a checkpoint only ever names ranges of that session's own journal.
//! Only branch-capable authorities (`SqliteHosting`) support them; the
//! volatile default refuses.
// THREAT[TM-DURABLE-015]: session-scoped records built by the authority from
// the session's own journal, non-destructive restore and branch deletion,
// bounded storage and catalog, display-safe previews.
use crate::Engine;
use crate::hosting::spi::events::{Event, EventData};
use crate::hosting::spi::session_commands::{SessionCommand, history};
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
/// Sequences read per call when collecting a checkpoint's last turn.
const TURN_PAGE: usize = 1024;

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
        // One authority transaction reads the branch, its events and the
        // input assignments and commits the checkpoint, so a concurrent
        // append or branch change cannot produce a checkpoint off its branch.
        let checkpoint = self
            .profile()?
            .create_conversation_checkpoint(self.session, label)
            .await?;
        Ok(CheckpointSummary {
            on_active_branch: true,
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
        if mode == RestoreMode::Branch {
            // Restoring to a point some retained branch already is reuses
            // it instead of adding another branch.
            if let Some(existing) = lineage
                .branches
                .iter()
                .find(|branch| branch.id != lineage.active && same(&branch.ranges, &target))
            {
                // `redo` re-reads the lineage and selects against that
                // revision, not the one `resolve` read. Benign: the reused
                // branch is identified by its ranges, and a concurrent
                // branch change still fails the revision check in `redo`.
                return self.redo(&existing.id.clone()).await;
            }
            if lineage.branches.len() >= history::MAX_BRANCHES {
                return Err(AgentLoopError::config(
                    "conversation branch capacity reached; delete retained branches with Checkpoints::delete_branch or restore with RestoreMode::Truncate",
                ));
            }
        }
        // A fresh identity never collides with a caller-chosen branch.
        let branch = format!("restore-{}", uuid::Uuid::new_v4().simple());
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

    /// Delete a retained, inactive branch, such as an undone tail that is no
    /// longer wanted. Its events stay in the journal and raw export;
    /// checkpoints only it held are no longer listed. `false` when it did
    /// not exist.
    pub async fn delete_branch(&self, branch: &str) -> Result<bool> {
        let profile = self.profile()?;
        let lineage = profile.history_lineage(self.session).await?;
        if branch == lineage.active {
            return Err(AgentLoopError::config(
                "the active history branch cannot be deleted",
            ));
        }
        if lineage.branch(branch).is_none() {
            return Ok(false);
        }
        profile
            .command(
                self.session,
                SessionCommand::DropBranch {
                    id: uuid::Uuid::new_v4().to_string(),
                    expected_revision: lineage.revision,
                    branch: branch.to_owned(),
                },
            )
            .await?;
        Ok(true)
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
            // Events of the last turn appended after the checkpoint (such as
            // `session.idled`) belong to it, however many there are. Page
            // until an empty page: an authority may return short pages.
            let mut after = checkpoint.through;
            loop {
                let sequences = profile
                    .turn_sequences(self.session, turn, after, TURN_PAGE)
                    .await?;
                target.extend(
                    sequences
                        .iter()
                        .filter(|sequence| contains(&branch, **sequence))
                        .map(|sequence| SequenceRange::new(*sequence, *sequence)),
                );
                match sequences.last() {
                    Some(last) => after = *last,
                    None => break,
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
}

/// Authoritative facts about one input message, read by the authority in
/// the transaction that commits a checkpoint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputFacts {
    /// Turn the authority assigned the input to (`assignment/{input}`),
    /// steered inputs included.
    pub assigned: Option<TurnId>,
    /// The input was admitted (it has an admission receipt); seeded history
    /// has none.
    pub admitted: bool,
    /// The input was rejected and is hidden from context reads.
    pub rejected: bool,
    /// The input is consumed history no turn answered: a cutover imported it
    /// with its admission receipt, but the legacy executor never ran it, so
    /// it is not queued work. It stays in checkpoints like seeded history.
    pub history: bool,
}

/// Builds a [`ConversationCheckpoint`] from one authority snapshot, for
/// hosting plugins implementing
/// [`SessionHosting::create_conversation_checkpoint`]: feed the active
/// branch's raw journal events in order, then [`build`](Self::build) with
/// the active ranges read in the same transaction.
#[derive(Default)]
pub struct CheckpointBuilder {
    fold: Fold,
}

impl CheckpointBuilder {
    /// An empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add the next active-branch event, in sequence order.
    pub fn add(&mut self, event: &Event) {
        self.fold.add(event);
    }

    /// The checkpoint at the end of the last finished turn. `facts` is asked
    /// only about input messages at or before that point that no included
    /// turn started. Membership comes from the authority's assignment, never
    /// from where an input happens to sit in the journal: an input admitted
    /// while one turn ran but answered by a later one stays out.
    pub fn build(
        self,
        active: &[SequenceRange],
        branch: &str,
        label: Option<String>,
        mut facts: impl FnMut(MessageId) -> Result<InputFacts>,
    ) -> Result<ConversationCheckpoint> {
        let fold = self.fold;
        let (through, last) = fold.boundary(active);
        let included: std::collections::HashSet<TurnId> = fold
            .turns
            .iter()
            .filter(|turn| turn.last <= through)
            .map(|turn| turn.id)
            .collect();
        let started: std::collections::HashSet<MessageId> = fold
            .turns
            .iter()
            .filter(|turn| included.contains(&turn.id))
            .filter_map(|turn| turn.message)
            .collect();
        let mut candidates: Vec<(MessageId, i32)> = fold
            .inputs
            .iter()
            .filter(|(id, (sequence, _))| *sequence <= through && !started.contains(*id))
            .map(|(id, (sequence, _))| (*id, *sequence))
            .collect();
        candidates.sort_unstable_by_key(|(_, sequence)| *sequence);
        let mut excluded = Vec::new();
        for (message, sequence) in candidates {
            let facts = facts(message)?;
            let keep = match facts.assigned {
                Some(turn) => included.contains(&turn),
                // A rejected input is hidden from context anyway; keeping
                // its sequence avoids splitting ranges for nothing. Only an
                // admitted live input without a turn is queued work.
                None => facts.rejected || !facts.admitted || facts.history,
            };
            if !keep {
                excluded.push(sequence);
            }
        }
        let ranges = subtract(&prefix(active, through), &excluded);
        if ranges.len() > history::MAX_RANGES {
            return Err(AgentLoopError::config(
                "conversation checkpoint would split history into too many ranges",
            ));
        }
        Ok(ConversationCheckpoint {
            id: uuid::Uuid::now_v7(),
            label,
            branch: branch.to_owned(),
            through,
            ranges,
            excluded,
            turn_index: included.len() as u32,
            turn_id: last.map(|turn| turn.id),
            message_id: last.and_then(|turn| turn.message),
            preview: last.and_then(|turn| fold.input(turn).and_then(|(_, text)| text.clone())),
            created_at: everruns_contracts::rt::utc_now().to_rfc3339(),
        })
    }
}

/// The retained branch a checkpoint lies on, the active one preferred;
/// `None` when no retained branch reaches it. Hosting plugins implementing
/// [`SessionHosting::create_conversation_checkpoint`] use it to prune
/// unreachable checkpoints in the committing transaction.
pub fn reachable<'a>(
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
    use crate::hosting::spi::RuntimeMessage;
    use crate::hosting::spi::events::{EventContext, InputMessageData, OutputMessageCompletedData};
    use everruns_contracts::runtime::events::TurnCompletedData;

    fn r(first: i32, last: i32) -> SequenceRange {
        SequenceRange::new(first, last)
    }

    fn at(sequence: i32, context: EventContext, data: impl Into<EventData>) -> Event {
        let mut event = Event::new(SessionId::new(), context, data);
        event.sequence = Some(sequence);
        event
    }

    fn input(sequence: i32, message: &RuntimeMessage) -> Event {
        at(
            sequence,
            EventContext::empty(),
            InputMessageData::new(message.clone()),
        )
    }

    fn reply(sequence: i32, turn: TurnId, message: MessageId) -> Event {
        at(
            sequence,
            EventContext::turn(turn, message),
            OutputMessageCompletedData::new(RuntimeMessage::assistant("Done.")),
        )
    }

    fn end(sequence: i32, turn: TurnId, message: MessageId) -> Event {
        at(
            sequence,
            EventContext::turn(turn, message),
            TurnCompletedData {
                turn_id: turn,
                ..Default::default()
            },
        )
    }

    /// Review finding: an input admitted while T1 runs but answered by T2
    /// sits inside T1's journal span. Membership must come from the
    /// assignment, not the span.
    #[test]
    fn inputs_follow_their_assignment_not_their_journal_position() {
        let (t1, t2) = (TurnId::new(), TurnId::new());
        let hello = RuntimeMessage::user("hello");
        let queued = RuntimeMessage::user("queued");
        let journal = [
            input(1, &hello),
            reply(2, t1, hello.id),
            input(3, &queued),
            end(4, t1, hello.id),
            reply(5, t2, queued.id),
        ];
        let build = |facts: InputFacts| {
            let mut builder = CheckpointBuilder::new();
            for event in &journal {
                builder.add(event);
            }
            let mut asked = Vec::new();
            let checkpoint = builder
                .build(&[r(1, 5)], "main", None, |message| {
                    asked.push(message);
                    Ok(facts)
                })
                .unwrap();
            assert_eq!(asked, [queued.id], "only unstarted inputs are looked up");
            checkpoint
        };
        let answered_later = build(InputFacts {
            assigned: Some(t2),
            admitted: true,
            ..Default::default()
        });
        assert_eq!(answered_later.through, 4);
        assert_eq!(answered_later.ranges, [r(1, 2), r(4, 4)]);
        assert_eq!(answered_later.excluded, [3]);
        assert_eq!(answered_later.turn_id, Some(t1));
        let pending = build(InputFacts {
            admitted: true,
            ..Default::default()
        });
        assert_eq!(pending.excluded, [3]);
        for kept in [
            InputFacts {
                assigned: Some(t1),
                admitted: true,
                ..Default::default()
            },
            InputFacts::default(),
            // Imported with a receipt but never run: consumed history.
            InputFacts {
                admitted: true,
                history: true,
                ..Default::default()
            },
            InputFacts {
                admitted: true,
                rejected: true,
                ..Default::default()
            },
        ] {
            let checkpoint = build(kept);
            assert_eq!(checkpoint.ranges, [r(1, 4)], "{kept:?}");
            assert!(checkpoint.excluded.is_empty());
        }
    }

    #[test]
    fn too_fragmented_checkpoints_fail_clearly() {
        let turn = TurnId::new();
        let start = RuntimeMessage::user("start");
        let mut journal = Vec::new();
        let mut pending = std::collections::HashSet::new();
        let mut sequence = 0;
        for _ in 0..history::MAX_RANGES + 1 {
            for admitted in [false, true] {
                sequence += 1;
                let message = RuntimeMessage::user("x");
                if admitted {
                    pending.insert(message.id);
                }
                journal.push(input(sequence, &message));
            }
        }
        journal.push(input(sequence + 1, &start));
        journal.push(end(sequence + 2, turn, start.id));
        let mut builder = CheckpointBuilder::new();
        for event in &journal {
            builder.add(event);
        }
        let error = builder
            .build(&[r(1, sequence + 2)], "main", None, |message| {
                Ok(InputFacts {
                    admitted: pending.contains(&message),
                    ..Default::default()
                })
            })
            .unwrap_err();
        assert!(error.to_string().contains("too many ranges"), "{error}");
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
