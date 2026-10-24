//! Work reports: what a piece of work is doing now, and what it got done.
//!
//! A work report is the one record every surface reads to show a turn's
//! progress and its result: the chat work log, the work card, the sessions
//! list, the API.
//!
//! Decisions:
//! - Built by the runtime from events the session already records, never
//!   authored by the model. Every turn of every agent gets one with no prompt
//!   change and no extra model call. The agent can enrich it (a plan through
//!   the plan tools), but is never required for it.
//! - One reducer, here. Surfaces render the report; they do not re-derive it,
//!   so live and replayed views cannot disagree.
//! - No invented progress. "Step N of M" exists only when the agent declared
//!   a plan; without one the report counts steps.
//! - Computed on read from the turn's events, so nothing new is stored and a
//!   report for an old turn reads the same as it did live.
//! - Labels come from tool narration, which already applies the redaction
//!   rules, so the report adds no new way to leak arguments.

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::tool_approval_types::{APPROVE_TOOL_CALL_TOOL, TOOL_APPROVAL_CALL_ID_PREFIX};

use super::background::{ProgressStep, ProgressStepStatus};
use super::events::{Event, EventData};
use super::session_task::TaskArtifact;

/// Event types a work report reads. Hosts load a turn's events of these types
/// and fold them in order.
pub const WORK_REPORT_EVENT_TYPES: &[&str] = &[
    super::events::TURN_STARTED,
    super::events::TURN_COMPLETED,
    super::events::TURN_FAILED,
    super::events::TURN_CANCELLED,
    super::events::TURN_SEALED,
    super::events::REASON_STARTED,
    super::events::TOOL_STARTED,
    super::events::TOOL_PROGRESS,
    super::events::TOOL_COMPLETED,
    super::events::TOOL_CALL_REQUESTED,
    super::events::FILE_WRITTEN,
    super::events::TASK_CREATED,
];

/// Tools that write the plan. Their calls shape [`WorkReport::plan`] and are
/// not listed as steps of their own.
pub const PLAN_TOOL_NAMES: &[&str] = &["write_todos", "update_checklist", "update_status"];

/// Newest steps kept in a report; older ones are counted in
/// [`WorkReport::omitted_steps`].
pub const MAX_REPORT_STEPS: usize = 50;

/// Written files listed as artifacts; the fact still counts all of them.
pub const MAX_FILE_ARTIFACTS: usize = 10;

/// Upper bound on [`WorkResult::summary`], in characters.
pub const MAX_SUMMARY_CHARS: usize = 280;

/// Where a piece of work stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WorkState {
    /// Running.
    Working,
    /// Parked on a person: an approval, a question, or another client action.
    NeedsYou,
    /// Finished.
    Done,
    /// Stopped by an error.
    Failed,
    /// Stopped by a person, or deliberately sealed to prevent waste.
    Stopped,
}

/// Where one step stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WorkStepState {
    /// Running now.
    Running,
    /// Finished.
    Done,
    /// Failed and went back to the model, which carried on. Not an error.
    Issue,
}

/// Live progress inside one step, reported by the tool while it runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WorkProgress {
    /// Items done so far, when the tool knows a count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 7))]
    pub current: Option<u64>,
    /// Items in total, when the tool knows a count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 12))]
    pub total: Option<u64>,
    /// The tool's latest status line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "Rendering image 7 of 12"))]
    pub label: Option<String>,
}

/// One step of the work: a tool call, labelled by its narration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WorkStep {
    /// The tool call id.
    #[cfg_attr(feature = "openapi", schema(example = "call_01"))]
    pub id: String,
    /// Tool name.
    #[cfg_attr(feature = "openapi", schema(example = "bash"))]
    pub tool_name: String,
    /// What the step is doing ("Running tests") or what came out of it
    /// ("Read AGENTS.md").
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Ran `cargo test`, 12 tests passed")
    )]
    pub label: String,
    /// Where the step stands.
    pub state: WorkStepState,
    /// When the step started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "2026-10-23T12:00:00Z"))]
    pub started_at: Option<DateTime<Utc>>,
    /// How long the step took, once finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 4200))]
    pub duration_ms: Option<u64>,
    /// Live progress while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<WorkProgress>,
}

/// The plan the agent shared, from its latest plan tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WorkPlan {
    /// Plan items in order.
    pub items: Vec<ProgressStep>,
    /// Items done or skipped.
    #[cfg_attr(feature = "openapi", schema(example = 2))]
    pub done: u32,
    /// Items in total.
    #[cfg_attr(feature = "openapi", schema(example = 4))]
    pub total: u32,
    /// The item in progress, in its "doing" form when the tool has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "Run the tests"))]
    pub current: Option<String>,
}

/// What the work waits on a person for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WorkAttentionKind {
    /// A gated tool call waits for approval.
    Approval,
    /// The agent asked a question.
    Question,
    /// Another client-side action, such as connecting an account.
    Action,
}

/// The open request a person has to answer. Answering still goes through the
/// endpoint that owns the request; the report only points at it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WorkAttention {
    /// What kind of answer is needed.
    pub kind: WorkAttentionKind,
    /// One line naming what is asked ("Run git push").
    #[cfg_attr(feature = "openapi", schema(example = "Deploy to staging"))]
    pub prompt: String,
    /// Open client-side call ids that carry the request.
    pub tool_call_ids: Vec<String>,
}

/// A measured fact about finished work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WorkFactKind {
    /// Distinct files written to the session filesystem.
    FilesChanged,
    /// Failed tool calls the work recovered from.
    Issues,
    /// Background tasks (subagents, threads, background tools) started.
    TasksStarted,
    /// Steps run.
    Steps,
}

/// One measured fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WorkFact {
    /// Which fact.
    pub kind: WorkFactKind,
    /// Its value.
    #[cfg_attr(feature = "openapi", schema(example = 3))]
    pub value: u64,
}

/// Where [`WorkResult::summary`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WorkSummarySource {
    /// The agent reported it.
    Agent,
    /// The first sentence of the agent's final reply.
    Reply,
}

/// What finished work produced.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WorkResult {
    /// One or two sentences on what changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Fixed the proration rounding and added a test.")
    )]
    pub summary: Option<String>,
    /// Where the summary came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_source: Option<WorkSummarySource>,
    /// Measured facts, in display order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<WorkFact>,
    /// Files, links, pull requests and sessions the work produced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<TaskArtifact>,
}

/// One piece of work, as every surface shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WorkReport {
    /// The turn the report describes.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "turn_01933b5a00007000800000000000001")
    )]
    pub turn_id: String,
    /// Where the work stands.
    pub state: WorkState,
    /// One line, always present: the current step while working, the request
    /// while waiting, the result when finished.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Step 3 of 4 · Running `cargo test`")
    )]
    pub headline: String,
    /// Agent name snapshot at turn start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "Billing fixer"))]
    pub agent_name: Option<String>,
    /// Model of the latest generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "gpt-6.1-sol"))]
    pub model: Option<String>,
    /// When the work started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "2026-10-23T12:00:00Z"))]
    pub started_at: Option<DateTime<Utc>>,
    /// When the work ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "2026-10-23T12:02:13Z"))]
    pub ended_at: Option<DateTime<Utc>>,
    /// Elapsed time: to the end when finished, to the read time while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 133000))]
    pub duration_ms: Option<u64>,
    /// The plan the agent shared, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<WorkPlan>,
    /// Steps in order, newest last, bounded by [`MAX_REPORT_STEPS`].
    pub steps: Vec<WorkStep>,
    /// Steps run but no longer listed.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "openapi", schema(example = 0))]
    pub omitted_steps: u32,
    /// The open request when the work waits on a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<WorkAttention>,
    /// What finished work produced. Absent while working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<WorkResult>,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl WorkReport {
    /// Builds the report of one turn from its events, oldest first. Events of
    /// other turns and other types are ignored. `now` stands in for the end
    /// of work still running. Returns `None` when no `turn.started` was seen.
    pub fn from_events<'a>(
        events: impl IntoIterator<Item = &'a Event>,
        now: DateTime<Utc>,
    ) -> Option<Self> {
        let mut builder = WorkReportBuilder::default();
        for event in events {
            builder.fold(event);
        }
        builder.build(now)
    }

    /// Value of one fact, if present.
    pub fn fact(&self, kind: WorkFactKind) -> Option<u64> {
        self.result
            .as_ref()?
            .facts
            .iter()
            .find(|fact| fact.kind == kind)
            .map(|fact| fact.value)
    }
}

#[derive(Debug, Clone)]
enum Terminal {
    Completed {
        final_answer_preview: Option<String>,
        issue_count: Option<u32>,
    },
    Failed,
    Cancelled,
    Sealed,
}

#[derive(Debug, Clone)]
struct PendingRequest {
    kind: WorkAttentionKind,
    prompt: String,
}

/// Incremental form of [`WorkReport::from_events`], for hosts that fold events
/// as they arrive.
#[derive(Debug, Clone, Default)]
pub struct WorkReportBuilder {
    turn_id: Option<String>,
    agent_name: Option<String>,
    model: Option<String>,
    started_at: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
    terminal: Option<Terminal>,
    steps: Vec<WorkStep>,
    step_index: HashMap<String, usize>,
    plan: Option<WorkPlan>,
    /// Plan tool calls seen started, so their completion is not a step and a
    /// failed plan update does not replace the plan.
    plan_calls: HashMap<String, WorkPlan>,
    /// Open client-side requests by call id, in arrival order.
    requests: Vec<(String, PendingRequest)>,
    /// Gated calls whose approval request is open or answered; not steps.
    gated: BTreeSet<String>,
    files: Vec<String>,
    tasks_started: u64,
    issues: u64,
}

impl WorkReportBuilder {
    /// Folds one event in. The first `turn.started` fixes the turn; events of
    /// other turns are ignored.
    pub fn fold(&mut self, event: &Event) {
        if let EventData::TurnStarted(data) = &event.data {
            if self.turn_id.is_none() {
                self.turn_id = Some(data.turn_id.to_string());
                self.agent_name = data.agent_name.clone();
                self.started_at = Some(event.ts);
            }
            return;
        }
        let Some(turn_id) = self.turn_id.as_deref() else {
            return;
        };
        if let Some(event_turn) = event.context.turn_id.as_ref()
            && event_turn.to_string() != turn_id
        {
            return;
        }
        match &event.data {
            EventData::ReasonStarted(data) => {
                if let Some(metadata) = &data.metadata {
                    self.model = Some(metadata.model.clone());
                }
            }
            EventData::ToolStarted(data) => {
                let call = &data.tool_call;
                if PLAN_TOOL_NAMES.contains(&call.name.as_str()) {
                    if let Some(plan) = plan_from_arguments(&call.name, &call.arguments) {
                        self.plan_calls.insert(call.id.clone(), plan);
                    }
                    return;
                }
                let label = data
                    .narration
                    .clone()
                    .or_else(|| data.display_name.clone())
                    .unwrap_or_else(|| call.name.clone());
                self.upsert_step(WorkStep {
                    id: call.id.clone(),
                    tool_name: call.name.clone(),
                    label,
                    state: WorkStepState::Running,
                    started_at: Some(event.ts),
                    duration_ms: None,
                    progress: None,
                });
            }
            EventData::ToolProgress(data) => {
                if let Some(&index) = self.step_index.get(&data.tool_call_id) {
                    let progress = self.steps[index]
                        .progress
                        .get_or_insert_with(WorkProgress::default);
                    progress.label = Some(data.message.clone());
                    // A count only means something as a pair, and a later
                    // uncounted update keeps the last known one.
                    if let (Some(current), Some(total)) = (data.current, data.total)
                        && total > 0
                    {
                        progress.current = Some(current.min(total));
                        progress.total = Some(total);
                    }
                }
            }
            EventData::ToolCompleted(data) => {
                if let Some(plan) = self.plan_calls.remove(&data.tool_call_id) {
                    if data.success {
                        self.plan = Some(plan);
                    }
                    return;
                }
                if PLAN_TOOL_NAMES.contains(&data.tool_name.as_str()) {
                    return;
                }
                if let Some(position) = self
                    .requests
                    .iter()
                    .position(|(id, _)| id == &data.tool_call_id)
                {
                    // An answered approval request is not a step: the model
                    // runs the gated call again, and that run is the step.
                    let (_, request) = self.requests.remove(position);
                    if request.kind == WorkAttentionKind::Approval {
                        return;
                    }
                }
                if self.gated.contains(&data.tool_call_id) || is_gated_result(data) {
                    // Parked on approval: the request that follows replaces
                    // the step, and the call did not run, so it is no issue.
                    return;
                }
                if !data.success {
                    self.issues += 1;
                }
                let label = data
                    .narration
                    .clone()
                    .or_else(|| data.display_name.clone())
                    .unwrap_or_else(|| data.tool_name.clone());
                let state = if data.success {
                    WorkStepState::Done
                } else {
                    WorkStepState::Issue
                };
                match self.step_index.get(&data.tool_call_id) {
                    Some(&index) => {
                        let step = &mut self.steps[index];
                        step.label = label;
                        step.state = state;
                        step.duration_ms = data.duration_ms;
                        step.progress = None;
                    }
                    None => self.upsert_step(WorkStep {
                        id: data.tool_call_id.clone(),
                        tool_name: data.tool_name.clone(),
                        label,
                        state,
                        started_at: None,
                        duration_ms: data.duration_ms,
                        progress: None,
                    }),
                }
            }
            EventData::ToolCallRequested(data) => {
                for call in &data.tool_calls {
                    let summary = data.tool_summaries.iter().find(|s| s.id == call.id);
                    let mut prompt = None;
                    let kind = if call.name == APPROVE_TOOL_CALL_TOOL {
                        if let Some(gated) = call.id.strip_prefix(TOOL_APPROVAL_CALL_ID_PREFIX) {
                            // The gated call's own narration names the action
                            // ("Run git push"); the request's names the approval.
                            prompt = self
                                .step_index
                                .get(gated)
                                .map(|&index| self.steps[index].label.clone());
                            self.gated.insert(gated.to_string());
                            self.remove_step(gated);
                        }
                        prompt = prompt.or_else(|| {
                            call.arguments
                                .get("display_name")
                                .or_else(|| call.arguments.get("tool"))
                                .and_then(|value| value.as_str())
                                .map(str::to_string)
                        });
                        WorkAttentionKind::Approval
                    } else if call.name == "ask_user" {
                        WorkAttentionKind::Question
                    } else {
                        WorkAttentionKind::Action
                    };
                    let prompt = prompt
                        .or_else(|| summary.and_then(|s| s.narration.clone()))
                        .or_else(|| summary.and_then(|s| s.display_name.clone()))
                        .or_else(|| data.headline.clone())
                        .unwrap_or_else(|| call.name.clone());
                    if !self.requests.iter().any(|(id, _)| id == &call.id) {
                        self.requests
                            .push((call.id.clone(), PendingRequest { kind, prompt }));
                    }
                }
            }
            EventData::FileWritten(data) => {
                if !self.files.contains(&data.path) {
                    self.files.push(data.path.clone());
                }
            }
            EventData::TaskCreated(_) => self.tasks_started += 1,
            EventData::TurnCompleted(data) => {
                self.finish(
                    event.ts,
                    Terminal::Completed {
                        final_answer_preview: data.final_answer_preview.clone(),
                        issue_count: data.issue_count,
                    },
                );
            }
            EventData::TurnFailed(_) => self.finish(event.ts, Terminal::Failed),
            EventData::TurnCancelled(_) => self.finish(event.ts, Terminal::Cancelled),
            EventData::TurnSealed(_) => self.finish(event.ts, Terminal::Sealed),
            _ => {}
        }
    }

    fn finish(&mut self, ts: DateTime<Utc>, terminal: Terminal) {
        if self.terminal.is_none() {
            self.terminal = Some(terminal);
            self.ended_at = Some(ts);
        }
    }

    fn upsert_step(&mut self, step: WorkStep) {
        match self.step_index.get(&step.id) {
            Some(&index) => self.steps[index] = step,
            None => {
                self.step_index.insert(step.id.clone(), self.steps.len());
                self.steps.push(step);
            }
        }
    }

    fn remove_step(&mut self, id: &str) {
        if let Some(index) = self.step_index.remove(id) {
            self.steps.remove(index);
            for value in self.step_index.values_mut() {
                if *value > index {
                    *value -= 1;
                }
            }
        }
    }

    /// The report so far. `now` stands in for the end of running work.
    pub fn build(&self, now: DateTime<Utc>) -> Option<WorkReport> {
        let turn_id = self.turn_id.clone()?;
        let end = self.ended_at.unwrap_or(now);
        let duration_ms = self
            .started_at
            .map(|start| (end - start).num_milliseconds().max(0) as u64);

        let finished = self.terminal.is_some();
        let mut steps = self.steps.clone();
        if finished {
            // A turn that ended stops its steps too: a step still marked
            // running was cut short, and listing it as running would lie.
            for step in &mut steps {
                if step.state == WorkStepState::Running {
                    step.state = WorkStepState::Issue;
                    step.progress = None;
                }
            }
        }
        let omitted_steps = steps.len().saturating_sub(MAX_REPORT_STEPS);
        if omitted_steps > 0 {
            steps.drain(..omitted_steps);
        }

        let attention = (!finished).then(|| self.attention()).flatten();
        let state = match (&self.terminal, &attention) {
            (Some(Terminal::Completed { .. }), _) => WorkState::Done,
            (Some(Terminal::Failed), _) => WorkState::Failed,
            (Some(Terminal::Cancelled | Terminal::Sealed), _) => WorkState::Stopped,
            (None, Some(_)) => WorkState::NeedsYou,
            (None, None) => WorkState::Working,
        };

        let result = finished.then(|| self.result());
        let headline = headline(
            state,
            duration_ms,
            &steps,
            self.plan.as_ref(),
            attention.as_ref(),
            result.as_ref(),
        );

        Some(WorkReport {
            turn_id,
            state,
            headline,
            agent_name: self.agent_name.clone(),
            model: self.model.clone(),
            started_at: self.started_at,
            ended_at: self.ended_at,
            duration_ms,
            plan: self.plan.clone(),
            steps,
            omitted_steps: omitted_steps as u32,
            attention,
            result,
        })
    }

    fn attention(&self) -> Option<WorkAttention> {
        let (_, first) = self.requests.first()?;
        let tool_call_ids = self
            .requests
            .iter()
            .filter(|(_, request)| request.kind == first.kind)
            .map(|(id, _)| id.clone())
            .collect();
        Some(WorkAttention {
            kind: first.kind,
            prompt: first.prompt.clone(),
            tool_call_ids,
        })
    }

    fn result(&self) -> WorkResult {
        let issues = match &self.terminal {
            Some(Terminal::Completed {
                issue_count: Some(count),
                ..
            }) => u64::from(*count),
            _ => self.issues,
        };
        let mut facts = Vec::new();
        if !self.files.is_empty() {
            facts.push(WorkFact {
                kind: WorkFactKind::FilesChanged,
                value: self.files.len() as u64,
            });
        }
        if self.tasks_started > 0 {
            facts.push(WorkFact {
                kind: WorkFactKind::TasksStarted,
                value: self.tasks_started,
            });
        }
        if issues > 0 {
            facts.push(WorkFact {
                kind: WorkFactKind::Issues,
                value: issues,
            });
        }
        facts.push(WorkFact {
            kind: WorkFactKind::Steps,
            value: self.steps.len() as u64,
        });

        let artifacts = self
            .files
            .iter()
            .take(MAX_FILE_ARTIFACTS)
            .map(|path| TaskArtifact {
                name: basename(path).to_string(),
                artifact_type: "file".to_string(),
                path: Some(path.clone()),
                url: None,
            })
            .collect();

        let summary = match &self.terminal {
            Some(Terminal::Completed {
                final_answer_preview: Some(preview),
                ..
            }) => first_sentence(preview, MAX_SUMMARY_CHARS),
            _ => None,
        };
        WorkResult {
            summary_source: summary.as_ref().map(|_| WorkSummarySource::Reply),
            summary,
            facts,
            artifacts,
        }
    }
}

/// Whether a failed call stopped on a hard approval gate. Matches the
/// structured payload only, so a tool whose output merely mentions the code
/// (a search through this repository) still counts as a step.
fn is_gated_result(data: &super::events::ToolCompletedData) -> bool {
    use super::message::ContentPart;
    use crate::tool_approval_types::TOOL_APPROVAL_REQUIRED_CODE;
    !data.success
        && data.result.iter().flatten().any(|part| match part {
            ContentPart::Text(text) => serde_json::from_str::<serde_json::Value>(&text.text)
                .ok()
                .and_then(|value| value.get("code")?.as_str().map(str::to_string))
                .is_some_and(|code| code == TOOL_APPROVAL_REQUIRED_CODE),
            _ => false,
        })
}

fn plan_from_arguments(tool: &str, arguments: &serde_json::Value) -> Option<WorkPlan> {
    let (key, title_key, active_key) = match tool {
        "write_todos" => ("todos", "content", Some("activeForm")),
        _ => ("steps", "title", None),
    };
    let entries = arguments.get(key)?.as_array()?;
    let mut items = Vec::with_capacity(entries.len());
    let mut current = None;
    for entry in entries {
        let Some(title) = entry.get(title_key).and_then(|value| value.as_str()) else {
            continue;
        };
        let status = match entry.get("status").and_then(|value| value.as_str()) {
            Some("in_progress") => ProgressStepStatus::InProgress,
            Some("completed" | "done") => ProgressStepStatus::Done,
            Some("skipped") => ProgressStepStatus::Skipped,
            _ => ProgressStepStatus::Pending,
        };
        if status == ProgressStepStatus::InProgress && current.is_none() {
            current = Some(
                active_key
                    .and_then(|key| entry.get(key))
                    .and_then(|value| value.as_str())
                    .unwrap_or(title)
                    .to_string(),
            );
        }
        items.push(ProgressStep {
            title: title.to_string(),
            status,
        });
    }
    if items.is_empty() {
        return None;
    }
    let done = items
        .iter()
        .filter(|item| {
            matches!(
                item.status,
                ProgressStepStatus::Done | ProgressStepStatus::Skipped
            )
        })
        .count() as u32;
    Some(WorkPlan {
        total: items.len() as u32,
        done,
        items,
        current,
    })
}

fn headline(
    state: WorkState,
    duration_ms: Option<u64>,
    steps: &[WorkStep],
    plan: Option<&WorkPlan>,
    attention: Option<&WorkAttention>,
    result: Option<&WorkResult>,
) -> String {
    let elapsed = duration_ms.map(format_duration);
    match state {
        WorkState::Working => steps
            .iter()
            .rev()
            .find(|step| step.state == WorkStepState::Running)
            .map(running_label)
            .or_else(|| plan.and_then(|plan| plan.current.clone()))
            .unwrap_or_else(|| "Working".to_string()),
        WorkState::NeedsYou => {
            let Some(attention) = attention else {
                return "Waiting for you".to_string();
            };
            let lead = match attention.kind {
                WorkAttentionKind::Approval => "Needs your approval",
                WorkAttentionKind::Question => "Waiting for your answer",
                WorkAttentionKind::Action => "Waiting for you",
            };
            format!("{lead} · {}", attention.prompt)
        }
        WorkState::Done => {
            let mut parts = vec![match elapsed {
                Some(elapsed) => format!("Done in {elapsed}"),
                None => "Done".to_string(),
            }];
            if let Some(result) = result {
                for fact in &result.facts {
                    match fact.kind {
                        WorkFactKind::FilesChanged => {
                            parts.push(count_phrase(fact.value, "file changed", "files changed"))
                        }
                        WorkFactKind::TasksStarted => {
                            parts.push(count_phrase(fact.value, "task started", "tasks started"))
                        }
                        WorkFactKind::Issues => {
                            parts.push(count_phrase(fact.value, "issue", "issues"))
                        }
                        WorkFactKind::Steps => {}
                    }
                }
            }
            parts.join(" · ")
        }
        WorkState::Failed => match elapsed {
            Some(elapsed) => format!("Failed after {elapsed}"),
            None => "Failed".to_string(),
        },
        WorkState::Stopped => match elapsed {
            Some(elapsed) => format!("Stopped after {elapsed}"),
            None => "Stopped".to_string(),
        },
    }
}

fn running_label(step: &WorkStep) -> String {
    match &step.progress {
        Some(WorkProgress {
            current: Some(current),
            total: Some(total),
            ..
        }) => format!("{} {current} of {total}", step.label),
        _ => step.label.clone(),
    }
}

fn count_phrase(value: u64, one: &str, many: &str) -> String {
    if value == 1 {
        format!("1 {one}")
    } else {
        format!("{value} {many}")
    }
}

/// "850ms", "13s", "2m 5s", "1h 3m".
pub fn format_duration(ms: u64) -> String {
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let seconds = ms / 1_000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let (minutes, seconds) = (seconds / 60, seconds % 60);
    if minutes < 60 {
        return if seconds == 0 {
            format!("{minutes}m")
        } else {
            format!("{minutes}m {seconds}s")
        };
    }
    let (hours, minutes) = (minutes / 60, minutes % 60);
    if minutes == 0 {
        format!("{hours}h")
    } else {
        format!("{hours}h {minutes}m")
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

/// The first sentence of a reply, on one line, bounded. Markdown emphasis and
/// heading marks are dropped, since the summary renders as plain text.
fn first_sentence(text: &str, max_chars: usize) -> Option<String> {
    let line = text
        .lines()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .find(|line| !line.is_empty())?;
    let line = line.replace("**", "").replace('`', "");
    let mut end = line.len();
    for (index, _) in line.match_indices(['.', '!', '?']) {
        let next = line[index + 1..].chars().next();
        if next.is_none_or(char::is_whitespace) {
            end = index + 1;
            break;
        }
    }
    let sentence = line[..end].trim();
    if sentence.is_empty() {
        return None;
    }
    if sentence.chars().count() <= max_chars {
        return Some(sentence.to_string());
    }
    let cut: String = sentence.chars().take(max_chars.saturating_sub(1)).collect();
    Some(format!("{}…", cut.trim_end()))
}

#[cfg(test)]
#[path = "work_report_tests.rs"]
mod tests;
