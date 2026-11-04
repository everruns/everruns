//! Self-healing (Evals rebuild step 11; knowledge/evaluation/self-healing.md).
//!
//! An org decides, per kind of action, whether Everruns takes it on its own
//! when a finding opens (`auto`), offers it on the finding for a person to
//! take (`suggest`), or does neither (`off`). Every action, automatic or not,
//! is recorded on the finding, announced to the people the finding notifies,
//! and can be undone from the finding.
//!
//! Decisions:
//! - Only protective, reversible actions can be automatic. Pausing a
//!   repeating schedule is one: the 48-run hard stop already does it, this
//!   acts at the warning count instead, and resuming puts the schedule back
//!   exactly as it was. It is `auto` by default.
//! - Changes to an agent (the suggested fixes of step 10) stay with a person:
//!   `agent_fixes` is `suggest` or `off`, never `auto`. Applying a change to
//!   an agent's instructions without a person reading it is the kind of
//!   self-modification the org has to opt into deliberately, which needs its
//!   own design (an automatic check budget, a revert path); until then the
//!   policy refuses it rather than accepting a value it cannot honour.
//! - `off` never disables a hard limit: a repeating schedule still stops at 48
//!   runs, a budget still stops at zero. It only turns off the earlier action.
//! - Platform-side abuse actions are never automatic on classifier evidence,
//!   and are not part of an org's policy at all.
use super::background_work::{self, SCHEDULE_RUNAWAY};
use crate::domains::agents::{AGENT_MANAGE, AGENT_VIEW};
use crate::domains::common::*;
use crate::domains::session_schedules::service::{ScheduleChange, SessionScheduleService};
use crate::storage::finding_actions::FindingActionRow;
use crate::storage::{SessionScheduleRow, StorageBackend};
use chrono::{DateTime, Utc};
use everruns_contracts::runtime::session_schedule::stop_reason;
use everruns_contracts::typed_id::ScheduleId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Pause a schedule whose runs keep ending the same way.
pub const PAUSE_SCHEDULE: &str = "pause_schedule";

/// What Everruns does about one kind of finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SelfHealingMode {
    /// Take the action as soon as the finding opens, and say so.
    Auto,
    /// Offer the action on the finding for a person to take.
    Suggest,
    /// Neither. Hard limits still apply.
    Off,
}

/// An org's self-healing policy (Settings → Limits).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct SelfHealingPolicy {
    /// Pausing a schedule whose runs keep ending the same way, at the warning
    /// count rather than the hard stop. Default `auto`.
    pub runaway_schedules: SelfHealingMode,
    /// The Inspector's suggested changes to an agent's instructions: `suggest`
    /// (default) or `off`. A person always approves them.
    pub agent_fixes: SelfHealingMode,
}

impl Default for SelfHealingPolicy {
    fn default() -> Self {
        Self {
            runaway_schedules: SelfHealingMode::Auto,
            agent_fixes: SelfHealingMode::Suggest,
        }
    }
}

impl SelfHealingPolicy {
    /// Reject a policy Everruns cannot honour.
    pub fn validate(&self) -> Result<(), String> {
        if self.agent_fixes == SelfHealingMode::Auto {
            return Err(
                "agent_fixes cannot be auto: a person approves every change to an agent"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// The org's policy; the defaults when it never set one or the stored value
/// no longer parses (logged).
pub async fn policy(db: &StorageBackend, org_id: i64) -> SelfHealingPolicy {
    match db.org_self_healing(org_id).await {
        Ok(Some(value)) => serde_json::from_value(value).unwrap_or_else(|error| {
            tracing::warn!(%error, org_id, "Stored self-healing policy does not parse; using defaults");
            SelfHealingPolicy::default()
        }),
        Ok(None) => SelfHealingPolicy::default(),
        Err(error) => {
            tracing::warn!(%error, org_id, "Could not read the self-healing policy; using defaults");
            SelfHealingPolicy::default()
        }
    }
}

/// Called by the scheduler when a schedule reaches the warning count. Pauses
/// it when the org's policy says `auto`; `true` when it did, and the fire
/// should not happen.
pub async fn pause_runaway(
    db: &Arc<StorageBackend>,
    schedule: &SessionScheduleRow,
    runs: i32,
) -> bool {
    if policy(db, schedule.org_id).await.runaway_schedules != SelfHealingMode::Auto {
        return false;
    }
    let service = SessionScheduleService::new(db.clone());
    if let Err(error) = service
        .stop(schedule.org_id, schedule.id, stop_reason::RUNAWAY)
        .await
    {
        tracing::warn!(%error, schedule_id = %schedule.id, "Could not pause a repeating schedule");
        return false;
    }
    tracing::info!(schedule_id = %schedule.id, runs, "Paused a repeating schedule under the org's self-healing policy");
    if let Some(issue) = background_work::observe_runaway_paused(db, schedule, runs).await
        && let Err(error) = db
            .record_finding_action(
                schedule.org_id,
                issue.id,
                issue.episode_id,
                PAUSE_SCHEDULE,
                &schedule.public_id,
                None,
            )
            .await
    {
        tracing::warn!(%error, schedule_id = %schedule.id, "Could not record the automatic pause");
    }
    true
}

/// An action taken on a finding.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FindingAction {
    /// Prefixed public identifier (`fact_…`).
    pub id: String,
    /// What was done: `pause_schedule`.
    pub kind: String,
    /// What it acted on, by public id (a schedule's `sched_…`).
    pub target_id: String,
    /// Whether Everruns took it on its own under the org's policy.
    pub automatic: bool,
    /// The finding episode it belongs to is the current one.
    pub current: bool,
    /// When it was taken.
    pub created_at: DateTime<Utc>,
    /// When it was undone, if it was.
    pub undone_at: Option<DateTime<Utc>>,
}

/// An action a person can take on a finding now.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AvailableFindingAction {
    /// What it would do: `pause_schedule`.
    pub kind: String,
    /// What it would act on, by public id.
    pub target_id: String,
}

/// What has been done about a finding, and what can be.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FindingActions {
    /// Actions taken, newest first.
    pub actions: Vec<FindingAction>,
    /// Actions the org's policy offers for a person to take now.
    pub available: Vec<AvailableFindingAction>,
    /// The org's policy for this kind of finding, when it has one.
    pub mode: Option<SelfHealingMode>,
}

fn present(row: FindingActionRow, episode_id: Uuid) -> FindingAction {
    FindingAction {
        id: row.public_id,
        kind: row.kind,
        target_id: row.target_id,
        automatic: row.taken_by.is_none(),
        current: row.episode_id == episode_id,
        created_at: row.created_at,
        undone_at: row.undone_at,
    }
}

fn schedule_of(issue: &crate::storage::HealthIssueRow) -> Option<String> {
    (issue.code == SCHEDULE_RUNAWAY)
        .then(|| issue.subject_id.clone())
        .flatten()
        .filter(|id| id.starts_with("sched_"))
}

async fn finding(
    ctx: &Ctx,
    issue_id: Uuid,
) -> Result<crate::storage::HealthIssueRow, CommandError> {
    ctx.db
        .get_health_issue(ctx.org_id(), issue_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Finding not found"))
}

pub(super) async fn actions_for(
    ctx: &Ctx,
    issue: &crate::storage::HealthIssueRow,
) -> Result<FindingActions, CommandError> {
    let rows = ctx.db.list_finding_actions(ctx.org_id(), issue.id).await?;
    let taken_now = rows
        .iter()
        .any(|r| r.episode_id == issue.episode_id && r.kind == PAUSE_SCHEDULE);
    let mode = schedule_of(issue).map(|_| SelfHealingMode::Auto);
    let mode = match mode {
        Some(_) => Some(policy(&ctx.db, ctx.org_id()).await.runaway_schedules),
        None => None,
    };
    let mut available = Vec::new();
    if let Some(schedule) = schedule_of(issue)
        && issue.status == "open"
        && !taken_now
        && mode != Some(SelfHealingMode::Off)
        && schedule_enabled(ctx, &schedule).await?
    {
        available.push(AvailableFindingAction {
            kind: PAUSE_SCHEDULE.into(),
            target_id: schedule,
        });
    }
    Ok(FindingActions {
        actions: rows
            .into_iter()
            .map(|row| present(row, issue.episode_id))
            .collect(),
        available,
        mode,
    })
}

async fn schedule_enabled(ctx: &Ctx, public_id: &str) -> Result<bool, CommandError> {
    let Ok(id) = ScheduleId::parse(public_id) else {
        return Ok(false);
    };
    Ok(ctx
        .db
        .get_session_schedule(ctx.org_id(), id)
        .await?
        .is_some_and(|s| s.enabled))
}

/// List what has been done about a finding.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListFindingActions {
    /// The finding.
    pub issue_id: Uuid,
}

#[command(
    name = "list_finding_actions",
    category = "evals",
    summary = "List actions on a finding",
    description = "List what Everruns or a person did about a finding, such as pausing a repeating schedule, and what the organization's self-healing policy offers to do now.",
    method = "GET",
    cli = CliRoute::new(&["health-issues", "actions"], "list").with_examples(&[CliExample::new("See whether a repeating schedule was paused", "everruns health-issues actions list --issue-id 550e8400-e29b-41d4-a716-446655440000")]),
    path = "/v1/health-issues/{issue_id}/actions",
    policy = AGENT_VIEW,
    http = plain,
)]
impl Command for ListFindingActions {
    type Output = FindingActions;

    async fn execute(self, ctx: &Ctx) -> Result<FindingActions, CommandError> {
        let issue = finding(ctx, self.issue_id).await?;
        actions_for(ctx, &issue).await
    }
}

/// The action to take.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct TakeFindingActionRequest {
    /// `pause_schedule`.
    pub kind: String,
}

/// Take an action the policy offers on a finding.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct TakeFindingAction {
    /// The finding.
    pub issue_id: Uuid,
    #[serde(flatten)]
    pub req: TakeFindingActionRequest,
}

#[command(
    name = "take_finding_action",
    category = "evals",
    summary = "Act on a finding",
    description = "Take an action the organization's self-healing policy offers on a finding, such as pausing a schedule whose runs keep ending the same way. It is recorded on the finding and can be undone.",
    method = "POST",
    cli = CliRoute::new(&["health-issues", "actions"], "take").with_examples(&[CliExample::new("Pause a repeating schedule from its finding", "everruns health-issues actions take --issue-id 550e8400-e29b-41d4-a716-446655440000 --kind pause_schedule --reason 'Claim expired, polling is pointless'")]),
    path = "/v1/health-issues/{issue_id}/actions",
    policy = AGENT_MANAGE,
    request_body(TakeFindingActionRequest),
    http = plain,
)]
impl Command for TakeFindingAction {
    type Output = FindingActions;

    async fn execute(self, ctx: &Ctx) -> Result<FindingActions, CommandError> {
        let issue = finding(ctx, self.issue_id).await?;
        if self.req.kind.trim() != PAUSE_SCHEDULE {
            return Err(CommandError::conflict(
                "That action is not available on this finding",
            ));
        }
        pause_from_finding(ctx, &issue).await?;
        actions_for(ctx, &issue).await
    }
}

/// Pause the schedule a runaway finding names, recorded as the finding's
/// action so it can be undone. Shared by the finding's actions and the
/// Health fixer (`fixers::SCHEDULE_PAUSE`).
pub(super) async fn pause_from_finding(
    ctx: &Ctx,
    issue: &crate::storage::HealthIssueRow,
) -> Result<(), CommandError> {
    let current = actions_for(ctx, issue).await?;
    let Some(action) = current.available.iter().find(|a| a.kind == PAUSE_SCHEDULE) else {
        return Err(CommandError::conflict(
            "That action is not available on this finding",
        ));
    };
    let schedule_id = ScheduleId::parse(&action.target_id)
        .map_err(|_| CommandError::unprocessable("The finding names no schedule"))?;
    let recorded = ctx
        .db
        .record_finding_action(
            ctx.org_id(),
            issue.id,
            issue.episode_id,
            PAUSE_SCHEDULE,
            &action.target_id,
            Some(ctx.caller.user_id.unwrap_or_default()),
        )
        .await?;
    if recorded.is_none() {
        return Err(CommandError::conflict("Someone already paused it"));
    }
    SessionScheduleService::new(ctx.db.clone())
        .stop(ctx.org_id(), schedule_id, stop_reason::RUNAWAY)
        .await?;
    Ok(())
}

/// Undo an action taken on a finding.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UndoFindingAction {
    /// The finding.
    pub issue_id: Uuid,
    /// Prefixed public identifier of the action (`fact_…`).
    pub action_id: String,
}

#[command(
    name = "undo_finding_action",
    category = "evals",
    summary = "Undo an action on a finding",
    description = "Undo an action taken on a finding, automatic or not. Undoing a pause resumes the schedule and starts its repeated-outcome count over.",
    method = "POST",
    cli = CliRoute::new(&["health-issues", "actions"], "undo").with_examples(&[CliExample::new("Resume a schedule that was paused automatically", "everruns health-issues actions undo --issue-id 550e8400-e29b-41d4-a716-446655440000 --action-id fact_01h9 --reason 'Still waiting on the claim; this is expected'")]),
    path = "/v1/health-issues/{issue_id}/actions/{action_id}/undo",
    policy = AGENT_MANAGE,
    http = plain,
)]
impl Command for UndoFindingAction {
    type Output = FindingActions;

    async fn execute(self, ctx: &Ctx) -> Result<FindingActions, CommandError> {
        let issue = finding(ctx, self.issue_id).await?;
        let action = ctx
            .db
            .get_finding_action(ctx.org_id(), self.action_id.trim())
            .await?
            .filter(|a| a.health_issue_id == issue.id)
            .ok_or_else(|| CommandError::not_found("Action not found"))?;
        if action.undone_at.is_some() {
            return Err(CommandError::conflict("It was already undone"));
        }
        let schedule_id = ScheduleId::parse(&action.target_id)
            .map_err(|_| CommandError::unprocessable("The action names no schedule"))?;
        // Claimed first so two people cannot both resume it.
        if !ctx
            .db
            .undo_finding_action(action.id, ctx.caller.user_id)
            .await?
        {
            return Err(CommandError::conflict("It was already undone"));
        }
        let resumed = SessionScheduleService::new(ctx.db.clone())
            .update(
                ctx.org_id(),
                schedule_id,
                ScheduleChange {
                    enabled: Some(true),
                    ..Default::default()
                },
            )
            .await;
        let failure = match resumed {
            Ok(Some(_)) => None,
            Ok(None) => Some(CommandError::not_found("The schedule no longer exists")),
            Err(Ok(message)) => Some(CommandError::unprocessable(message)),
            Err(Err(error)) => Some(CommandError::from(error)),
        };
        if let Some(error) = failure {
            ctx.db.redo_finding_action(action.id).await?;
            return Err(error);
        }
        actions_for(ctx, &issue).await
    }
}
