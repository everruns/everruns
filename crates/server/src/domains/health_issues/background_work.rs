//! Health issues for unattended background work (knowledge/operations/background-limits.md,
//! "Detection"): a schedule whose runs keep ending the same way, a schedule
//! stopped by its budget, an org whose daily unattended budget is spent, an
//! agent whose unattended spend jumped, and a session whose model calls send
//! a large, uncached history.
//!
//! Decisions:
//! - "Ended the same way" compares the final answers of consecutive fires,
//!   lowercased with digits and whitespace collapsed, so a changing timestamp
//!   or counter in an otherwise identical answer still counts as the same.
//! - The scheduler counts the previous fire's outcome just before the next
//!   fire: the count needs no turn-completion hook, and a run still going
//!   when the next fire is due is simply not counted yet.
//! - People hear about an issue when it opens (and again when a warned
//!   schedule is stopped): the schedule's owner and the agent's authors, the
//!   people who changed it, for agent and schedule issues; the org's owners
//!   for the org budget, and when an agent has no known author.
//! - Notifications carry no text from the agent's answers: they go to people
//!   who may not be able to open the session, and an answer can hold secrets.
//! - The sweep reports; only the runaway stop pauses anything.

use crate::storage::StorageBackend;
use crate::storage::background_work::{ScheduleWatchRow, SubjectHealthIssue};
use crate::storage::{HealthIssueRow, SessionScheduleRow};
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::SessionId;
use everruns_core::session_schedule::stop_reason;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// A schedule whose runs keep ending the same way.
pub const SCHEDULE_RUNAWAY: &str = "schedule.runaway";
/// A schedule stopped because its budget is spent.
pub const SCHEDULE_BUDGET: &str = "schedule.budget_exhausted";
/// The org's unattended budget for today is spent.
pub const ORG_UNATTENDED_BUDGET: &str = "org.unattended_budget";
/// An agent's unattended spend jumped above its recent days.
pub const SPEND_SPIKE: &str = "agent.unattended_spend_spike";
/// A session's model calls send a large history with little of it cached.
pub const CONTEXT_BLOAT: &str = "session.context_bloat";

/// Identical runs in a row after which each fire warns the agent.
pub const RUNAWAY_WARN_RUNS: i32 = 24;
/// Identical runs in a row after which the schedule stops.
pub const RUNAWAY_STOP_RUNS: i32 = 48;
/// An agent's unattended spend over a day must pass this, in USD...
const SPIKE_FLOOR_USD: f64 = 5.0;
/// ...and this many times its average day over the six days before.
const SPIKE_FACTOR: f64 = 3.0;
/// Model calls a session's context size is judged over.
const BLOAT_CALLS: i64 = 20;
/// Average prompt tokens per call above which a session is large...
const BLOAT_PROMPT_TOKENS: f64 = 200_000.0;
/// ...when less than this share of them was cached.
const BLOAT_CACHED_RATIO: f64 = 0.10;

const SWEEP_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Whether `code` is one of these detectors.
pub fn owns(code: &str) -> bool {
    matches!(
        code,
        SCHEDULE_RUNAWAY | SCHEDULE_BUDGET | ORG_UNATTENDED_BUDGET | SPEND_SPIKE | CONTEXT_BLOAT
    )
}

/// The final answer's text, normalized and hashed: equal for answers that
/// differ only in digits, case and spacing. `None` for an answer with no text.
pub fn outcome_fingerprint(content: &serde_json::Value) -> Option<String> {
    let mut text = String::new();
    collect_text(content, &mut text);
    let mut normalized = String::with_capacity(text.len());
    let mut last = ' ';
    for ch in text.chars().flat_map(char::to_lowercase) {
        let ch = if ch.is_ascii_digit() {
            '#'
        } else if ch.is_whitespace() {
            ' '
        } else {
            ch
        };
        if (ch == ' ' || ch == '#') && ch == last {
            continue;
        }
        normalized.push(ch);
        last = ch;
    }
    let normalized = normalized.trim();
    if normalized.is_empty() {
        return None;
    }
    Some(hex::encode(Sha256::digest(normalized.as_bytes())))
}

fn collect_text(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::String(text) => {
            out.push_str(text);
            out.push(' ');
        }
        serde_json::Value::Array(parts) => {
            for part in parts {
                collect_text(part, out);
            }
        }
        serde_json::Value::Object(part) => {
            if let Some(text) = part.get("text") {
                collect_text(text, out);
            }
        }
        _ => {}
    }
}

/// Count how the schedule's previous fire ended, and return how many runs in
/// a row have ended that way. Best effort: on any error the count is 0, so a
/// failing lookup never stops a schedule.
pub async fn count_previous_outcome(db: &StorageBackend, schedule: &SessionScheduleRow) -> i32 {
    let counted = async {
        let Some(input) = db.latest_schedule_fire(schedule.id.uuid()).await? else {
            return Ok(0);
        };
        let output = db
            .turn_final_output(schedule.session_id.uuid(), input)
            .await?;
        match output.as_ref().and_then(outcome_fingerprint) {
            Some(fingerprint) => {
                db.record_schedule_outcome(schedule.id.uuid(), input, &fingerprint)
                    .await
            }
            // Still running, or ended without an answer: not counted.
            None => db.schedule_same_outcome_runs(schedule.id.uuid()).await,
        }
    };
    let result: anyhow::Result<i32> = counted.await;
    result.unwrap_or_else(|error| {
        tracing::warn!(%error, schedule_id = %schedule.id, "Could not count the schedule's last outcome");
        0
    })
}

/// The note added to a fire's message once its runs keep ending the same way.
pub fn runaway_warning(runs: i32, next_trigger_at: Option<DateTime<Utc>>) -> String {
    let left = RUNAWAY_STOP_RUNS - runs;
    let when = next_trigger_at
        .map(|next| {
            let interval = next - Utc::now();
            let stop_at = Utc::now() + interval * left.max(1);
            format!(", around {}", stop_at.format("%Y-%m-%d %H:%M UTC"))
        })
        .unwrap_or_default();
    format!(
        "Warning: the last {runs} runs of this schedule ended the same way. If {left} more do, it stops automatically{when}. If waiting no longer helps, cancel it with cancel_schedule; if you still need to wait, create a schedule with a longer interval instead."
    )
}

/// Open or update the runaway issue for a schedule that was warned
/// (`stopped` false) or stopped.
pub async fn observe_runaway(
    db: &StorageBackend,
    schedule: &SessionScheduleRow,
    runs: i32,
    stopped: bool,
) {
    let Some(watch) = watch(db, &schedule.public_id).await else {
        return;
    };
    let evidence = serde_json::json!({
        "runs": runs,
        "stop_runs": RUNAWAY_STOP_RUNS,
        "stopped": stopped,
        "trigger_count": watch.trigger_count,
    });
    report_schedule(db, &watch, SCHEDULE_RUNAWAY, "open", evidence, stopped).await;
}

/// Open the issue for a schedule its budget stopped.
pub async fn observe_schedule_budget_stop(db: &StorageBackend, schedule: &SessionScheduleRow) {
    let Some(watch) = watch(db, &schedule.public_id).await else {
        return;
    };
    let budget = db
        .schedule_budgets(watch.org_id, std::slice::from_ref(&watch.public_id))
        .await
        .ok()
        .and_then(|rows| rows.into_iter().next());
    let evidence = serde_json::json!({
        "trigger_count": watch.trigger_count,
        "limit_usd": budget.as_ref().map(|b| b.limit),
        "spent_usd": budget.as_ref().map(|b| b.limit - b.balance),
    });
    report_schedule(db, &watch, SCHEDULE_BUDGET, "open", evidence, false).await;
}

/// Open the org's daily-budget issue when its unattended budget is spent.
pub async fn observe_org_unattended_spent(db: &StorageBackend, org_id: i64, limit_usd: f64) {
    let org = everruns_core::org_public_id_from_internal(org_id);
    let issue = SubjectHealthIssue {
        org_id,
        code: ORG_UNATTENDED_BUDGET,
        subject_id: &org,
        agent_id: None,
        session_id: None,
        status: "open",
        evidence: serde_json::json!({"limit_usd": limit_usd}),
        checked_at: Utc::now(),
    };
    let recipients = db.org_owner_user_ids(org_id).await.unwrap_or_default();
    record(db, issue, recipients, None, false).await;
}

async fn watch(db: &StorageBackend, public_id: &str) -> Option<ScheduleWatchRow> {
    match db.schedule_watch(public_id).await {
        Ok(watch) => watch,
        Err(error) => {
            tracing::warn!(%error, schedule = public_id, "Could not read the schedule for its health issue");
            None
        }
    }
}

async fn report_schedule(
    db: &StorageBackend,
    watch: &ScheduleWatchRow,
    code: &str,
    status: &str,
    mut evidence: serde_json::Value,
    resurface: bool,
) {
    evidence["schedule_id"] = watch.public_id.clone().into();
    evidence["description"] = excerpt(&watch.description, 120).into();
    let issue = SubjectHealthIssue {
        org_id: watch.org_id,
        code,
        subject_id: &watch.public_id,
        agent_id: watch.agent_id,
        session_id: Some(watch.session_id),
        status,
        evidence,
        checked_at: Utc::now(),
    };
    let recipients = if status == "open" {
        schedule_recipients(db, watch).await
    } else {
        Vec::new()
    };
    let agent = watch.agent_public_id.clone().zip(watch.agent_name.clone());
    record(db, issue, recipients, agent, resurface).await;
}

/// Record an observation and tell `recipients` when it opened an issue (or,
/// with `resurface`, made an open one worse).
async fn record(
    db: &StorageBackend,
    issue: SubjectHealthIssue<'_>,
    recipients: Vec<Uuid>,
    agent: Option<(String, String)>,
    resurface: bool,
) {
    let org_id = issue.org_id;
    let code = issue.code.to_string();
    let evidence = issue.evidence.clone();
    let observed = match db.observe_subject_health_issue(issue).await {
        Ok(Some(observed)) => observed,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, code, org_id, "Could not record a background-work health issue");
            return;
        }
    };
    if observed.status != "open" || !(observed.opened || resurface) || recipients.is_empty() {
        return;
    }
    let (title, body) = copy_for(
        &code,
        "open",
        &evidence,
        agent.as_ref().map(|a| a.1.as_str()),
    );
    let href = format!("/settings/health?issue={}", observed.id);
    if let Err(error) = db
        .announce_health_issue(
            org_id,
            &observed,
            &recipients,
            &title,
            &body,
            &href,
            agent
                .as_ref()
                .map(|(id, name)| (id.as_str(), name.as_str())),
            resurface,
        )
        .await
    {
        tracing::warn!(%error, code, org_id, "Could not announce a background-work health issue");
    }
}

/// The schedule's owner and the agent's authors; the org's owners when
/// neither is known.
async fn schedule_recipients(db: &StorageBackend, watch: &ScheduleWatchRow) -> Vec<Uuid> {
    let mut users = Vec::new();
    if let Some(owner) = watch.owner_user_id
        && db.is_org_member(watch.org_id, owner).await.unwrap_or(false)
    {
        users.push(owner);
    }
    if let Some(agent) = watch.agent_id {
        users.extend(
            db.agent_author_user_ids(watch.org_id, agent)
                .await
                .unwrap_or_default(),
        );
    }
    with_owner_fallback(db, watch.org_id, users).await
}

async fn agent_recipients(db: &StorageBackend, org_id: i64, agent_id: Option<Uuid>) -> Vec<Uuid> {
    let users = match agent_id {
        Some(agent) => db
            .agent_author_user_ids(org_id, agent)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };
    with_owner_fallback(db, org_id, users).await
}

async fn with_owner_fallback(db: &StorageBackend, org_id: i64, mut users: Vec<Uuid>) -> Vec<Uuid> {
    if users.is_empty() {
        users = db.org_owner_user_ids(org_id).await.unwrap_or_default();
    }
    users.sort();
    users.dedup();
    users
}

fn excerpt(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max {
        return text;
    }
    let cut: String = text.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// Title and body of a background-work issue in `status`.
pub fn copy(row: &HealthIssueRow) -> (String, String) {
    let evidence = row.evidence.clone().unwrap_or_default();
    copy_for(&row.code, &row.status, &evidence, row.agent_name.as_deref())
}

fn copy_for(
    code: &str,
    status: &str,
    evidence: &serde_json::Value,
    agent: Option<&str>,
) -> (String, String) {
    let agent = agent.unwrap_or("An agent");
    let task = evidence["description"]
        .as_str()
        .map(|task| format!(" (\"{task}\")"))
        .unwrap_or_default();
    let money = |key: &str| evidence[key].as_f64().map(|v| format!("${v:.2}"));
    if status == "resolved" {
        let title = match code {
            SCHEDULE_RUNAWAY => "Schedule no longer repeating",
            SCHEDULE_BUDGET => "Schedule budget handled",
            ORG_UNATTENDED_BUDGET => "Unattended spend back under today's limit",
            SPEND_SPIKE => "Unattended spend back to normal",
            _ => "Session context back to normal",
        };
        return (title.into(), "Nothing needs doing.".into());
    }
    match code {
        SCHEDULE_RUNAWAY if evidence["stopped"].as_bool() == Some(true) => (
            "Schedule stopped: it kept repeating".into(),
            format!(
                "{agent}'s schedule{task} ended the same way {} runs in a row, so it was stopped. Open the session to resume it, extend it, or let it go.",
                evidence["runs"]
                    .as_i64()
                    .unwrap_or(RUNAWAY_STOP_RUNS as i64)
            ),
        ),
        SCHEDULE_RUNAWAY => (
            "Schedule keeps repeating".into(),
            format!(
                "{agent}'s schedule{task} has ended the same way {} runs in a row. It stops by itself after {RUNAWAY_STOP_RUNS}. Open the session to cancel it if waiting no longer helps.",
                evidence["runs"]
                    .as_i64()
                    .unwrap_or(RUNAWAY_WARN_RUNS as i64)
            ),
        ),
        SCHEDULE_BUDGET => (
            "Schedule stopped at its budget".into(),
            format!(
                "{agent}'s schedule{task} spent {} of its {} budget and was stopped. Open the session to raise its budget and resume it, or let it go.",
                money("spent_usd").unwrap_or_else(|| "all".into()),
                money("limit_usd").unwrap_or_default(),
            ),
        ),
        ORG_UNATTENDED_BUDGET => (
            "Daily unattended spend limit reached".into(),
            format!(
                "Schedules and triggered sessions in this organization spent today's {} limit. Schedules skip their runs until midnight UTC. Raise or remove the limit in Settings → Limits.",
                money("limit_usd").unwrap_or_default()
            ),
        ),
        SPEND_SPIKE => (
            "Unattended spend jumped".into(),
            format!(
                "{agent} spent {} on unattended work in the last 24 hours, against {} on an average day before. Check its schedules and triggers.",
                money("last_day_usd").unwrap_or_default(),
                money("baseline_daily_usd").unwrap_or_default(),
            ),
        ),
        _ => (
            "Session sends a large, uncached history".into(),
            format!(
                "{agent}'s recent model calls in one session averaged {} prompt tokens with {}% cached, so every turn pays for the whole history again. Consider starting a new session or compacting this one.",
                evidence["avg_prompt_tokens"]
                    .as_f64()
                    .map(|v| format!("{:.0}k", v / 1000.0))
                    .unwrap_or_default(),
                evidence["cached_percent"]
                    .as_f64()
                    .map(|v| format!("{v:.0}"))
                    .unwrap_or_default(),
            ),
        ),
    }
}

/// Re-evaluate one issue now ("Check again").
pub async fn recheck(db: &StorageBackend, row: &HealthIssueRow) -> anyhow::Result<()> {
    match row.code.as_str() {
        SCHEDULE_RUNAWAY | SCHEDULE_BUDGET => recheck_schedule(db, row).await,
        ORG_UNATTENDED_BUDGET => sweep_org_budgets(db).await,
        SPEND_SPIKE => sweep_spend_spikes(db).await,
        CONTEXT_BLOAT => sweep_context_bloat(db).await,
        _ => Ok(()),
    }
}

/// Resolve a schedule issue once the schedule no longer has the problem: a
/// person resumed or canceled it, or its runs started ending differently.
async fn recheck_schedule(db: &StorageBackend, row: &HealthIssueRow) -> anyhow::Result<()> {
    let Some(subject) = row.subject_id.as_deref() else {
        return Ok(());
    };
    let watch = db.schedule_watch(subject).await?;
    let still = watch.as_ref().is_some_and(|w| {
        !w.deleted
            && match row.code.as_str() {
                SCHEDULE_RUNAWAY => {
                    (w.enabled && w.same_outcome_runs >= RUNAWAY_WARN_RUNS)
                        || (!w.enabled && w.stop_reason.as_deref() == Some(stop_reason::RUNAWAY))
                }
                _ => !w.enabled && w.stop_reason.as_deref() == Some(stop_reason::BUDGET_EXHAUSTED),
            }
    });
    if !still {
        resolve(db, row).await?;
    }
    Ok(())
}

async fn resolve(db: &StorageBackend, row: &HealthIssueRow) -> anyhow::Result<()> {
    let Some(subject) = row.subject_id.as_deref() else {
        return Ok(());
    };
    db.observe_subject_health_issue(SubjectHealthIssue {
        org_id: row.org_id,
        code: &row.code,
        subject_id: subject,
        agent_id: None,
        session_id: None,
        status: "resolved",
        evidence: row.evidence.clone().unwrap_or_default(),
        checked_at: Utc::now(),
    })
    .await?;
    Ok(())
}

async fn sweep_open_schedules(db: &StorageBackend) -> anyhow::Result<()> {
    for code in [SCHEDULE_RUNAWAY, SCHEDULE_BUDGET] {
        for row in db.open_subject_health_issues(code).await? {
            recheck_schedule(db, &row).await?;
        }
    }
    Ok(())
}

async fn sweep_org_budgets(db: &StorageBackend) -> anyhow::Result<()> {
    let spent = db.orgs_with_spent_unattended_budget().await?;
    for (org_id, limit) in &spent {
        observe_org_unattended_spent(db, *org_id, *limit).await;
    }
    for row in db.open_subject_health_issues(ORG_UNATTENDED_BUDGET).await? {
        if !spent.iter().any(|(org, _)| *org == row.org_id) {
            resolve(db, &row).await?;
        }
    }
    Ok(())
}

async fn sweep_spend_spikes(db: &StorageBackend) -> anyhow::Result<()> {
    let spikes = db
        .unattended_spend_spikes(SPIKE_FLOOR_USD, SPIKE_FACTOR)
        .await?;
    let mut seen = Vec::with_capacity(spikes.len());
    for spike in &spikes {
        let Some((public_id, name)) = db.agent_label(spike.agent_id).await? else {
            continue;
        };
        let issue = SubjectHealthIssue {
            org_id: spike.org_id,
            code: SPEND_SPIKE,
            subject_id: &public_id,
            agent_id: Some(spike.agent_id),
            session_id: None,
            status: "open",
            evidence: serde_json::json!({
                "last_day_usd": spike.last_day_usd,
                "baseline_daily_usd": spike.baseline_daily_usd,
            }),
            checked_at: Utc::now(),
        };
        let recipients = agent_recipients(db, spike.org_id, Some(spike.agent_id)).await;
        record(
            db,
            issue,
            recipients,
            Some((public_id.clone(), name)),
            false,
        )
        .await;
        seen.push((spike.org_id, public_id));
    }
    for row in db.open_subject_health_issues(SPEND_SPIKE).await? {
        if !seen
            .iter()
            .any(|(org, id)| *org == row.org_id && Some(id.as_str()) == row.subject_id.as_deref())
        {
            resolve(db, &row).await?;
        }
    }
    Ok(())
}

async fn sweep_context_bloat(db: &StorageBackend) -> anyhow::Result<()> {
    let sessions = db
        .bloated_sessions(BLOAT_CALLS, BLOAT_PROMPT_TOKENS, BLOAT_CACHED_RATIO)
        .await?;
    let mut seen = Vec::with_capacity(sessions.len());
    for session in &sessions {
        let subject = SessionId::from_uuid(session.session_id).to_string();
        let agent = match session.agent_id {
            Some(agent) => db.agent_label(agent).await?,
            None => None,
        };
        let issue = SubjectHealthIssue {
            org_id: session.org_id,
            code: CONTEXT_BLOAT,
            subject_id: &subject,
            agent_id: session.agent_id,
            session_id: Some(session.session_id),
            status: "open",
            evidence: serde_json::json!({
                "calls": session.calls,
                "avg_prompt_tokens": session.avg_prompt_tokens,
                "cached_percent": session.cached_ratio * 100.0,
            }),
            checked_at: Utc::now(),
        };
        let recipients = agent_recipients(db, session.org_id, session.agent_id).await;
        record(db, issue, recipients, agent, false).await;
        seen.push((session.org_id, subject));
    }
    // A session leaves the list once its calls are back under the bar or it
    // has had none in the last hour.
    for row in db.open_subject_health_issues(CONTEXT_BLOAT).await? {
        if !seen
            .iter()
            .any(|(org, id)| *org == row.org_id && Some(id.as_str()) == row.subject_id.as_deref())
        {
            resolve(db, &row).await?;
        }
    }
    Ok(())
}

/// One pass of every background-work detector.
pub async fn sweep(db: &StorageBackend) {
    if let Err(error) = sweep_open_schedules(db).await {
        tracing::warn!(%error, "Schedule health sweep failed");
    }
    if let Err(error) = sweep_org_budgets(db).await {
        tracing::warn!(%error, "Unattended budget health sweep failed");
    }
    if let Err(error) = sweep_spend_spikes(db).await {
        tracing::warn!(%error, "Unattended spend health sweep failed");
    }
    if let Err(error) = sweep_context_bloat(db).await {
        tracing::warn!(%error, "Context size health sweep failed");
    }
}

/// Run [`sweep`] every 15 minutes.
pub fn spawn_sweep(db: Arc<StorageBackend>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            sweep(&db).await;
            // Spread replicas' sweeps.
            let jitter = Uuid::new_v4().as_u128() % 61;
            tokio::time::sleep(SWEEP_INTERVAL + Duration::from_secs(jitter as u64)).await;
        }
    })
}
