//! Suggested fixes (Evals rebuild step 10; knowledge/evaluation/suggested-fixes.md).
//!
//! The Inspector writes a fix with its write-up when a short addition to the
//! agent's instructions would have prevented what the finding shows. A person
//! checks it, which runs one of the agent's evals as an experiment with and
//! without the addition, and applies it once the check shows no regressions.
//!
//! Decisions:
//! - Only instruction additions for now: the change is plain text a person
//!   can read in full, and undoing it is restoring the previous agent
//!   revision. Tool, model and schedule changes come with self-healing.
//! - The check is started by a person, not by the Inspector: an experiment
//!   costs model calls, and findings come from work that is often already
//!   expensive. Applying requires a check whose result is no regressions.
//! - The finding itself is not added to the dataset by the check: the dataset
//!   belongs to the people who keep it. The check proves the fix does no harm
//!   on the cases they keep, and reports any case it fixed.
//! - The variant with the fix uses the eval target's `system_prompt`, which
//!   is placed before the agent's instructions; applying appends the addition
//!   to them. Either way the agent reads the same text alongside its own.
//! - A fix is applied only to the instructions it was checked against: a
//!   check records their digest, and applying refuses when they changed since.
//! - Suspected abuse never gets a fix: helping a misused agent work better is
//!   the opposite of what that finding is for.
use super::{background_work, classifiers};
use crate::domains::agents::types::UpdateAgentRequest;
use crate::domains::agents::{AGENT_MANAGE, UpdateAgentCmd};
use crate::domains::common::*;
use crate::domains::evals::experiments::{
    self,
    types::{
        CreateEvalExperimentRequest, EvalExperiment, ExperimentStatus, ExperimentVariantSpec,
        Verdict,
    },
};
use crate::domains::evals::record::EvalTarget;
use crate::domains::evals::{EVAL_RUN, EVAL_VIEW, queries as q};
use crate::storage::finding_fixes::FindingFixRow;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::AgentId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

/// Findings the Inspector may propose a fix for.
pub const FIXABLE: [&str; 3] = [
    classifiers::FOLLOWED_INJECTION,
    classifiers::AGENT_STUCK,
    background_work::SCHEDULE_RUNAWAY,
];

pub const SUMMARY_CHARS: usize = 160;
pub const ADDITION_CHARS: usize = 1500;
const NOTE_CHARS: usize = 500;
/// Labels of the check's two variants.
pub const CURRENT: &str = "current";
pub const WITH_FIX: &str = "with fix";
const DEFAULT_CHECK_TRIALS: u32 = 2;
const LIST_LIMIT: i64 = 50;

/// Statuses a person can still act on.
const OPEN: [&str; 4] = ["proposed", "verified", "regressed", "check_failed"];

pub fn instructions_sha256(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// The agent's instructions with the fix added.
pub fn apply_to(instructions: &str, addition: &str) -> String {
    let base = instructions.trim_end();
    if base.is_empty() {
        addition.trim().to_string()
    } else {
        format!("{base}\n\n{}", addition.trim())
    }
}

/// A suggested change to an agent's instructions, written by the Inspector for
/// a finding, and where checking and approving it stand.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FindingFix {
    #[schema(example = "fix_0190f3a1b2c37d8e9f00112233445566")]
    pub id: String,
    /// The finding the fix is for.
    pub issue_id: Uuid,
    #[schema(example = "agent_550e8400e29b41d4a716446655440000")]
    pub agent_id: String,
    /// One line on what the change does.
    pub summary: String,
    /// The text added to the end of the agent's instructions.
    pub addition: String,
    /// `proposed` (not checked), `checking`, `verified` (checked, no
    /// regressions), `regressed`, `check_failed`, `applied` or `rejected`.
    pub status: String,
    /// False when the agent's instructions changed after the last check, so
    /// the fix needs checking again before it can be applied.
    pub current: bool,
    /// The last check's result.
    pub check: Option<FixCheck>,
    /// The check's experiment, while or after it runs.
    pub eval_id: Option<String>,
    pub experiment_id: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    /// Why it was rejected.
    pub decision_note: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// What a check found: the eval's cases with and without the fix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FixCheck {
    pub eval_name: String,
    pub dataset_version: u32,
    pub trials: u32,
    /// Cases that ran on both sides.
    pub cases: u32,
    pub current: FixCheckSide,
    pub with_fix: FixCheckSide,
    /// Cases the fix made pass.
    pub fixed: u32,
    /// Cases the fix made fail.
    pub broke: u32,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FixCheckSide {
    /// Cases that passed every trial.
    pub passed: u32,
    /// Share of all trials that passed, 0 to 1.
    pub pass_rate: f64,
    pub cost_usd: f64,
}

/// Where a check's experiment stands.
#[derive(Debug, PartialEq)]
pub enum CheckOutcome {
    Running,
    Failed,
    Done(FixCheck),
}

/// Read a finished check from its experiment.
pub fn judge(experiment: &EvalExperiment, eval_name: &str) -> CheckOutcome {
    match experiment.status {
        ExperimentStatus::Running => return CheckOutcome::Running,
        ExperimentStatus::Failed | ExperimentStatus::Cancelled => return CheckOutcome::Failed,
        ExperimentStatus::Completed => {}
    }
    let Some(comparison) = &experiment.comparison else {
        return CheckOutcome::Failed;
    };
    let Some(verdict) = comparison.verdicts.iter().find(|v| v.label == WITH_FIX) else {
        return CheckOutcome::Failed;
    };
    let side = |label: &str| {
        let mut passed = 0;
        let (mut ok, mut trials, mut cost) = (0u32, 0u32, 0.0);
        for case in &comparison.cases {
            if let Some(cell) = case.cells.iter().find(|c| c.label == label) {
                if cell.trials > 0 && cell.passed == cell.trials {
                    passed += 1;
                }
                ok += cell.passed;
                trials += cell.trials;
                cost += cell.cost_usd;
            }
        }
        FixCheckSide {
            passed,
            pass_rate: if trials == 0 {
                0.0
            } else {
                f64::from(ok) / f64::from(trials)
            },
            cost_usd: cost,
        }
    };
    if verdict.paired_cases == 0 {
        return CheckOutcome::Failed;
    }
    CheckOutcome::Done(FixCheck {
        eval_name: eval_name.to_string(),
        dataset_version: experiment.dataset_version,
        trials: experiment.trials,
        cases: verdict.paired_cases,
        current: side(CURRENT),
        with_fix: side(WITH_FIX),
        fixed: verdict.fixed,
        broke: verdict.broke,
        verdict: verdict.verdict,
    })
}

/// The status a finished check leaves a fix in.
pub fn status_after(check: &FixCheck) -> &'static str {
    if check.broke == 0 && check.verdict != Verdict::Regression {
        "verified"
    } else {
        "regressed"
    }
}

/// Settle a running check when its experiment has finished.
async fn refresh(ctx: &Ctx, row: FindingFixRow) -> Result<FindingFixRow, CommandError> {
    if row.status != "checking" {
        return Ok(row);
    }
    let (Some(eval_id), Some(experiment_id)) = (&row.eval_public_id, &row.experiment_public_id)
    else {
        ctx.db
            .move_finding_fix(
                row.id,
                &["checking"],
                "check_failed",
                None,
                None,
                None,
                None,
            )
            .await?;
        return reload(ctx, &row.public_id).await;
    };
    let Some(experiment) = experiments::get(&ctx.db, &ctx.caller, eval_id, experiment_id)
        .await
        .map_err(classify_anyhow)?
    else {
        return Ok(row);
    };
    let eval_name = ctx
        .db
        .get_eval_by_public_id(ctx.org_id(), eval_id)
        .await?
        .map(|e| e.name)
        .unwrap_or_default();
    let (to, check) = match judge(&experiment, &eval_name) {
        CheckOutcome::Running => return Ok(row),
        CheckOutcome::Failed => ("check_failed", None),
        CheckOutcome::Done(check) => (status_after(&check), Some(check)),
    };
    let check = check.map(|c| serde_json::to_value(c).unwrap_or_default());
    ctx.db
        .move_finding_fix(row.id, &["checking"], to, None, check.as_ref(), None, None)
        .await?;
    reload(ctx, &row.public_id).await
}

async fn reload(ctx: &Ctx, public_id: &str) -> Result<FindingFixRow, CommandError> {
    ctx.db
        .get_finding_fix(ctx.org_id(), public_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Fix not found"))
}

async fn agent_instructions(ctx: &Ctx, row: &FindingFixRow) -> Result<String, CommandError> {
    Ok(ctx
        .db
        .get_agent(ctx.org_id(), AgentId::from_uuid(row.agent_id))
        .await?
        .ok_or_else(|| CommandError::not_found("The agent no longer exists"))?
        .system_prompt)
}

pub async fn present(ctx: &Ctx, row: FindingFixRow) -> Result<FindingFix, CommandError> {
    let row = refresh(ctx, row).await?;
    let current = match ctx
        .db
        .get_agent(ctx.org_id(), AgentId::from_uuid(row.agent_id))
        .await?
    {
        Some(agent) => instructions_sha256(&agent.system_prompt) == row.base_instructions_sha256,
        None => false,
    };
    Ok(FindingFix {
        id: row.public_id,
        issue_id: row.health_issue_id,
        agent_id: row.agent_public_id,
        summary: row.summary,
        addition: row.addition,
        status: row.status,
        current,
        check: row
            .check_result
            .and_then(|c| serde_json::from_value(c).ok()),
        eval_id: row.eval_public_id,
        experiment_id: row.experiment_public_id,
        decided_at: row.decided_at,
        decision_note: row.decision_note,
        created_at: row.created_at,
    })
}

async fn fix(ctx: &Ctx, fix_id: &str) -> Result<FindingFixRow, CommandError> {
    reload(ctx, fix_id.trim()).await
}

fn bounded(text: &str, max: usize, what: &str) -> Result<String, CommandError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(CommandError::unprocessable(format!("{what} is empty")));
    }
    if text.chars().count() > max {
        return Err(CommandError::unprocessable(format!(
            "Keep {} under {max} characters",
            what.to_lowercase()
        )));
    }
    Ok(text.to_string())
}

/// The fixes list.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FindingFixList {
    pub fixes: Vec<FindingFix>,
}

/// List suggested fixes, for one finding or one agent.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListFindingFixes {
    /// The finding whose fix to show.
    pub issue_id: Option<Uuid>,
    /// An agent whose fixes to list.
    pub agent_id: Option<String>,
    /// Only fixes still waiting on a person (agent lists only).
    #[serde(default)]
    pub open: bool,
}

#[command(
    name = "list_finding_fixes",
    category = "evals",
    summary = "List suggested fixes",
    description = "List the fixes the Inspector suggested for one finding or for one agent's findings, with where checking and approving each stands.",
    method = "GET",
    cli = CliRoute::new(&["evals", "fixes"], "list").with_examples(&[CliExample::new("Fixes for one agent that still need a decision", "everruns evals fixes list --agent-id agent_01h9 --open")]),
    path = "/v1/evals/fixes",
    policy = EVAL_VIEW,
    http = plain,
)]
impl Command for ListFindingFixes {
    type Output = FindingFixList;

    async fn execute(self, ctx: &Ctx) -> Result<FindingFixList, CommandError> {
        let rows = match (self.issue_id, self.agent_id.as_deref()) {
            (Some(issue_id), None) => {
                let Some(issue) = ctx.db.get_health_issue(ctx.org_id(), issue_id).await? else {
                    return Err(CommandError::not_found("Finding not found"));
                };
                ctx.db
                    .finding_fix_for(issue.id, issue.episode_id)
                    .await?
                    .into_iter()
                    .collect()
            }
            (None, Some(agent_id)) => {
                let agent_id: AgentId = agent_id
                    .parse()
                    .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))?;
                let agent = ctx
                    .db
                    .get_agent(ctx.org_id(), agent_id)
                    .await?
                    .ok_or_else(|| CommandError::not_found("Agent not found"))?;
                ctx.db
                    .list_agent_finding_fixes(ctx.org_id(), agent.id.uuid(), self.open, LIST_LIMIT)
                    .await?
            }
            _ => {
                return Err(CommandError::bad_request(
                    "Give either issue_id or agent_id",
                ));
            }
        };
        let mut fixes = Vec::with_capacity(rows.len());
        for row in rows {
            fixes.push(present(ctx, row).await?);
        }
        Ok(FindingFixList { fixes })
    }
}

/// Options for checking a fix.
#[derive(Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct CheckFindingFixRequest {
    /// The agent's eval to check against. Default: the one with most cases.
    pub eval_id: Option<String>,
    /// Trials per case and side, 1 to 20 (default 2).
    pub trials: Option<u32>,
    /// Spending cap in USD for the whole check.
    pub max_cost_usd: Option<f64>,
}

/// Check a fix: run one of the agent's evals with and without it.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CheckFindingFix {
    pub fix_id: String,
    #[serde(flatten)]
    pub req: CheckFindingFixRequest,
}

#[command(
    name = "check_finding_fix",
    category = "evals",
    summary = "Check a suggested fix",
    description = "Run one of the agent's evals as an experiment with and without a suggested fix. The fix can be applied once the check shows no regressions.",
    method = "POST",
    cli = CliRoute::new(&["evals", "fixes"], "check").with_examples(&[CliExample::new("Check a fix on the agent's main eval within $2", "everruns evals fixes check --fix-id fix_01h9 --max-cost-usd 2 --reason 'Before applying the stop rule'")]),
    path = "/v1/evals/fixes/{fix_id}/check",
    policy = EVAL_RUN,
    request_body(CheckFindingFixRequest),
    http = plain,
)]
impl Command for CheckFindingFix {
    type Output = FindingFix;

    async fn execute(self, ctx: &Ctx) -> Result<FindingFix, CommandError> {
        if !ctx.feature_flags.evals {
            return Err(CommandError::feature_not_enabled("evals"));
        }
        let row = fix(ctx, &self.fix_id).await?;
        if !OPEN.contains(&row.status.as_str()) {
            return Err(CommandError::conflict(format!(
                "The fix is {} and cannot be checked",
                row.status.replace('_', " ")
            )));
        }
        let evals = ctx
            .db
            .agent_evals_with_cases(ctx.org_id(), row.agent_id)
            .await?;
        let chosen = match self.req.eval_id.as_deref().map(str::trim) {
            Some(wanted) => evals
                .into_iter()
                .find(|(id, _, _)| id == wanted)
                .ok_or_else(|| {
                    CommandError::unprocessable(
                        "That eval has no cases or does not test this agent",
                    )
                })?,
            None => evals.into_iter().next().ok_or_else(|| {
                CommandError::unprocessable(
                    "This agent has no eval with cases to check the fix against; \
                     add cases first (the Eval Builder can propose them)",
                )
            })?,
        };
        let eval = ctx
            .db
            .get_eval_by_public_id(ctx.org_id(), &chosen.0)
            .await?
            .ok_or_else(|| CommandError::not_found("Eval not found"))?;
        let target: EvalTarget = eval
            .target
            .clone()
            .and_then(|t| serde_json::from_value(t).ok())
            .ok_or_else(|| CommandError::unprocessable("The eval has no session target"))?;
        let EvalTarget::Session {
            harness_id,
            harness_name,
            agent_id,
            model_id,
            system_prompt,
            max_iterations,
        } = target
        else {
            return Err(CommandError::unprocessable(
                "The eval does not run sessions with this agent",
            ));
        };
        let with_fix = EvalTarget::Session {
            harness_id,
            harness_name,
            agent_id,
            model_id,
            system_prompt: Some(apply_to(
                system_prompt.as_deref().unwrap_or_default(),
                &row.addition,
            )),
            max_iterations,
        };
        let instructions = agent_instructions(ctx, &row).await?;
        let mut name: String = format!("Fix: {}", row.summary).chars().take(200).collect();
        name = name.trim().to_string();
        let experiment = experiments::create(
            &q::service(ctx),
            &ctx.db,
            &ctx.caller,
            &eval.public_id,
            CreateEvalExperimentRequest {
                name: Some(name),
                variants: vec![
                    ExperimentVariantSpec {
                        label: CURRENT.into(),
                        target: None,
                        model_override: None,
                    },
                    ExperimentVariantSpec {
                        label: WITH_FIX.into(),
                        target: Some(with_fix),
                        model_override: None,
                    },
                ],
                trials: Some(self.req.trials.unwrap_or(DEFAULT_CHECK_TRIALS)),
                max_cost_usd: self.req.max_cost_usd,
            },
        )
        .await
        .map_err(classify_anyhow)?;
        let experiment_id = ctx
            .db
            .get_eval_experiment_by_public_id(ctx.org_id(), &experiment.id)
            .await?
            .map(|e| e.id);
        ctx.db
            .rebase_finding_fix(row.id, &instructions_sha256(&instructions))
            .await?;
        if !ctx
            .db
            .move_finding_fix(row.id, &OPEN, "checking", experiment_id, None, None, None)
            .await?
        {
            return Err(CommandError::conflict(
                "Someone else changed the fix meanwhile",
            ));
        }
        present(ctx, reload(ctx, &row.public_id).await?).await
    }
}

/// New text for a fix. It must be checked again.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateFindingFixRequest {
    /// The text to add to the agent's instructions.
    pub addition: String,
    /// One line on what the change does. Unchanged when omitted.
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateFindingFix {
    pub fix_id: String,
    #[serde(flatten)]
    pub req: UpdateFindingFixRequest,
}

#[command(
    name = "update_finding_fix",
    category = "evals",
    summary = "Edit a suggested fix",
    description = "Change the text a suggested fix adds to the agent's instructions. An edited fix must be checked again before it can be applied.",
    method = "PATCH",
    cli = CliRoute::new(&["evals", "fixes"], "update").with_examples(&[CliExample::new("Tighten a fix before checking it", "everruns evals fixes update --fix-id fix_01h9 --addition 'Stop after 12 runs with the same result.' --reason 'Shorter stop rule'")]),
    path = "/v1/evals/fixes/{fix_id}",
    policy = AGENT_MANAGE,
    request_body(UpdateFindingFixRequest),
    http = plain,
)]
impl Command for UpdateFindingFix {
    type Output = FindingFix;

    async fn execute(self, ctx: &Ctx) -> Result<FindingFix, CommandError> {
        let row = fix(ctx, &self.fix_id).await?;
        let addition = bounded(&self.req.addition, ADDITION_CHARS, "The addition")?;
        let summary = match self.req.summary.as_deref() {
            Some(summary) => bounded(summary, SUMMARY_CHARS, "The summary")?,
            None => row.summary.clone(),
        };
        if !ctx
            .db
            .edit_finding_fix(row.id, &OPEN, &summary, &addition, ctx.caller.user_id)
            .await?
        {
            return Err(CommandError::conflict(format!(
                "The fix is {} and cannot be edited",
                row.status.replace('_', " ")
            )));
        }
        present(ctx, reload(ctx, &row.public_id).await?).await
    }
}

/// Apply a checked fix to the agent's instructions.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ApplyFindingFix {
    pub fix_id: String,
}

#[command(
    name = "apply_finding_fix",
    category = "evals",
    summary = "Apply a suggested fix",
    description = "Add a checked fix to the agent's instructions as a new agent revision. Only a fix whose check showed no regressions, against the instructions the agent has now, can be applied.",
    method = "POST",
    cli = CliRoute::new(&["evals", "fixes"], "apply").with_examples(&[CliExample::new("Apply a fix after its check passed", "everruns evals fixes apply --fix-id fix_01h9 --reason 'Stop rule checked on the support dataset'")]),
    path = "/v1/evals/fixes/{fix_id}/apply",
    policy = AGENT_MANAGE,
    http = plain,
)]
impl Command for ApplyFindingFix {
    type Output = FindingFix;

    async fn execute(self, ctx: &Ctx) -> Result<FindingFix, CommandError> {
        let row = refresh(ctx, fix(ctx, &self.fix_id).await?).await?;
        if row.status != "verified" {
            return Err(CommandError::conflict(match row.status.as_str() {
                "proposed" | "check_failed" => "Check the fix before applying it".to_string(),
                "checking" => "The check is still running".to_string(),
                "regressed" => "The check found regressions; edit the fix and check again".into(),
                other => format!("The fix is already {other}"),
            }));
        }
        let instructions = agent_instructions(ctx, &row).await?;
        if instructions_sha256(&instructions) != row.base_instructions_sha256 {
            return Err(CommandError::conflict(
                "The agent's instructions changed after the check; check the fix again",
            ));
        }
        // Claimed first so two approvals cannot both add the text.
        if !ctx
            .db
            .move_finding_fix(
                row.id,
                &["verified"],
                "applied",
                None,
                None,
                ctx.caller.user_id,
                None,
            )
            .await?
        {
            return Err(CommandError::conflict(
                "Someone else changed the fix meanwhile",
            ));
        }
        // The agent change goes through update_agent, which records the new
        // revision in the agent's history with this approval's reason.
        let mut inner = ctx.clone();
        let fallback = format!("Applied suggested fix {}", row.public_id);
        let reason = ctx
            .change_intent
            .as_ref()
            .and_then(|i| i.reason.clone())
            .unwrap_or(fallback);
        inner.change_intent = Some(
            crate::domains::change_history::ChangeIntent::default()
                .with_reason(Some(reason))
                .or(ctx.change_intent.as_ref()),
        );
        let applied = UpdateAgentCmd {
            id: row.agent_public_id.clone(),
            req: UpdateAgentRequest {
                system_prompt: Some(apply_to(&instructions, &row.addition)),
                ..Default::default()
            },
        }
        .run(&inner)
        .await;
        if let Err(error) = applied {
            ctx.db
                .move_finding_fix(row.id, &["applied"], "verified", None, None, None, None)
                .await?;
            return Err(error);
        }
        present(ctx, reload(ctx, &row.public_id).await?).await
    }
}

/// Why a fix is not wanted.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RejectFindingFixRequest {
    /// Kept so later suggestions can be compared with what people turned down.
    /// (Named `note` because `reason` is the change-intent flag every command
    /// takes.)
    pub note: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RejectFindingFix {
    pub fix_id: String,
    #[serde(flatten)]
    pub req: RejectFindingFixRequest,
}

#[command(
    name = "reject_finding_fix",
    category = "evals",
    summary = "Reject a suggested fix",
    description = "Turn down a suggested fix, saying why. The agent is not changed.",
    method = "POST",
    cli = CliRoute::new(&["evals", "fixes"], "reject").with_examples(&[CliExample::new("Turn down a fix that would hide the real problem", "everruns evals fixes reject --fix-id fix_01h9 --note 'The schedule itself is wrong, fixing it there'")]),
    path = "/v1/evals/fixes/{fix_id}/reject",
    policy = AGENT_MANAGE,
    request_body(RejectFindingFixRequest),
    http = plain,
)]
impl Command for RejectFindingFix {
    type Output = FindingFix;

    async fn execute(self, ctx: &Ctx) -> Result<FindingFix, CommandError> {
        let row = fix(ctx, &self.fix_id).await?;
        let note = bounded(&self.req.note, NOTE_CHARS, "The note")?;
        let mut from = OPEN.to_vec();
        from.push("checking");
        if !ctx
            .db
            .move_finding_fix(
                row.id,
                &from,
                "rejected",
                None,
                None,
                ctx.caller.user_id,
                Some(&note),
            )
            .await?
        {
            return Err(CommandError::conflict(format!(
                "The fix is already {}",
                row.status.replace('_', " ")
            )));
        }
        present(ctx, reload(ctx, &row.public_id).await?).await
    }
}
