//! Health issues about how a session behaves, built-in for every org (Evals
//! rebuild, detection layer 1; knowledge/operations/background-limits.md,
//! "Detection"): a tool call that keeps failing, an agent repeating the same
//! tool call, and a session spending far above its agent's usual hour.
//!
//! Decisions:
//! - "The same tool call" is the same tool on the same target (what the trace
//!   says the call acted on). Judged over the session's last tool calls of
//!   the last hour, so a long session's old history does not count.
//! - The spend baseline is the agent's own median session-hour over the week
//!   before, not a fixed amount: a research agent that always spends $3 an
//!   hour is normal, a support agent jumping from $0.05 to $3 is not. A median
//!   is used rather than a mean so one past runaway does not raise the bar. No
//!   verdict until the agent has enough history.
//! - These detectors report and notify; they stop nothing. Fixed caps (the
//!   budgets) do the stopping.
//! - Notifications name the tool and amounts, never the call's target or the
//!   agent's words: those can hold secrets, and the people told may not be able
//!   to open the session.
//! - Eval sessions are skipped (their storage queries filter them out).

use super::background_work::{agent_recipients, record, resolve};
use crate::storage::StorageBackend;
use crate::storage::background_work::SubjectHealthIssue;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::SessionId;
use std::sync::Mutex;

/// The same tool call keeps failing.
pub const TOOL_ERROR_LOOP: &str = "session.tool_error_loop";
/// The agent keeps making the same tool call.
pub const REPEATED_TOOL_CALLS: &str = "session.repeated_tool_calls";
/// A session spends far above its agent's usual hour.
pub const SESSION_SPEND_SPIKE: &str = "session.spend_spike";

/// Recent tool calls a session is judged over.
pub const TOOL_WINDOW: i64 = 40;
/// Failures of one call in the window that make a loop.
pub const LOOP_FAILURES: i64 = 8;
/// Repeats of one call in the window (half of it) that make a loop.
pub const REPEAT_CALLS: i64 = 20;
/// A session's spend over the last hour must pass this, in USD...
pub const SPEND_FLOOR_USD: f64 = 2.0;
/// ...and this many times its agent's median session-hour.
pub const SPEND_FACTOR: f64 = 10.0;
/// Session-hours of history an agent needs before it is judged.
pub const SPEND_MIN_SAMPLES: i64 = 12;

/// Every code these detectors open.
pub const CODES: [&str; 3] = [TOOL_ERROR_LOOP, REPEATED_TOOL_CALLS, SESSION_SPEND_SPIKE];

/// Whether `code` is one of these detectors.
pub fn owns(code: &str) -> bool {
    CODES.contains(&code)
}

/// Which issue a session's most repeated tool call is, if any.
pub fn classify_tool_pattern(calls: i64, failed: i64) -> Option<&'static str> {
    if failed >= LOOP_FAILURES {
        Some(TOOL_ERROR_LOOP)
    } else if calls >= REPEAT_CALLS {
        Some(REPEATED_TOOL_CALLS)
    } else {
        None
    }
}

/// Title and body of one of these issues in `status`.
pub fn copy_for(
    code: &str,
    status: &str,
    evidence: &serde_json::Value,
    agent: Option<&str>,
) -> (String, String) {
    let agent = agent.unwrap_or("An agent");
    let tool = evidence["tool"].as_str().unwrap_or("a tool");
    let count = |key: &str| evidence[key].as_i64().unwrap_or_default();
    let money = |key: &str| {
        evidence[key]
            .as_f64()
            .map(|v| format!("${v:.2}"))
            .unwrap_or_default()
    };
    if status == "resolved" {
        let title = match code {
            SESSION_SPEND_SPIKE => "Session spend back to normal",
            _ => "Tool calls back to normal",
        };
        return (title.into(), "Nothing needs doing.".into());
    }
    match code {
        TOOL_ERROR_LOOP => (
            "Tool keeps failing".into(),
            format!(
                "{agent} made the same {tool} call {} times in the last hour and it failed {} times. The session is probably stuck. Open the session to stop it, or fix what the tool needs.",
                count("calls"),
                count("failed"),
            ),
        ),
        REPEATED_TOOL_CALLS => (
            "Agent repeats the same tool call".into(),
            format!(
                "{agent} made the same {tool} call {} times among its last {} tool calls. It may be going in circles. Open the session to check on it.",
                count("calls"),
                count("window_calls"),
            ),
        ),
        _ => (
            "Session spending far above normal".into(),
            format!(
                "{agent} spent {} in one session over the last hour, against {} in its usual session-hour. Open the session to check what it is doing.",
                money("last_hour_usd"),
                money("baseline_hourly_usd"),
            ),
        ),
    }
}

/// One pass of these detectors.
pub async fn sweep(db: &StorageBackend) -> anyhow::Result<()> {
    sweep_tool_patterns(db).await?;
    sweep_spend(db).await
}

async fn sweep_tool_patterns(db: &StorageBackend) -> anyhow::Result<()> {
    let rows = db
        .repeated_tool_calls(TOOL_WINDOW, LOOP_FAILURES, REPEAT_CALLS)
        .await?;
    let mut seen: Vec<(&'static str, i64, String)> = Vec::with_capacity(rows.len());
    for row in &rows {
        let Some(code) = classify_tool_pattern(row.calls, row.failed) else {
            continue;
        };
        let evidence = serde_json::json!({
            "tool": row.tool,
            "calls": row.calls,
            "failed": row.failed,
            "window_calls": row.window_calls,
        });
        let subject =
            observe_session(db, code, row.org_id, row.session_id, row.agent_id, evidence).await?;
        seen.push((code, row.org_id, subject));
    }
    for code in [TOOL_ERROR_LOOP, REPEATED_TOOL_CALLS] {
        resolve_unseen(db, code, &seen).await?;
    }
    Ok(())
}

async fn sweep_spend(db: &StorageBackend) -> anyhow::Result<()> {
    let rows = db
        .session_spend_spikes(SPEND_FLOOR_USD, SPEND_FACTOR, SPEND_MIN_SAMPLES)
        .await?;
    let mut seen = Vec::with_capacity(rows.len());
    for row in &rows {
        let evidence = serde_json::json!({
            "last_hour_usd": row.last_hour_usd,
            "baseline_hourly_usd": row.baseline_hourly_usd,
            "samples": row.samples,
        });
        let subject = observe_session(
            db,
            SESSION_SPEND_SPIKE,
            row.org_id,
            row.session_id,
            Some(row.agent_id),
            evidence,
        )
        .await?;
        seen.push((SESSION_SPEND_SPIKE, row.org_id, subject));
    }
    resolve_unseen(db, SESSION_SPEND_SPIKE, &seen).await
}

/// Record an open observation about one session and tell the agent's
/// authors (or the org's owners) when it opened an issue. Returns the subject.
pub(super) async fn observe_session(
    db: &StorageBackend,
    code: &'static str,
    org_id: i64,
    session_id: uuid::Uuid,
    agent_id: Option<uuid::Uuid>,
    evidence: serde_json::Value,
) -> anyhow::Result<String> {
    let subject = SessionId::from_uuid(session_id).to_string();
    let agent = match agent_id {
        Some(agent) => db.agent_label(agent).await?,
        None => None,
    };
    let issue = SubjectHealthIssue {
        org_id,
        code,
        subject_id: &subject,
        agent_id,
        session_id: Some(session_id),
        status: "open",
        evidence,
        checked_at: Utc::now(),
    };
    let recipients = agent_recipients(db, org_id, agent_id).await;
    record(db, issue, recipients, agent, false).await;
    Ok(subject)
}

/// Resolve the open issues of `code` the sweep no longer sees.
async fn resolve_unseen(
    db: &StorageBackend,
    code: &str,
    seen: &[(&'static str, i64, String)],
) -> anyhow::Result<()> {
    for row in db.open_subject_health_issues(code).await? {
        let still = seen.iter().any(|(c, org, subject)| {
            *c == code && *org == row.org_id && Some(subject.as_str()) == row.subject_id.as_deref()
        });
        if !still {
            resolve(db, &row).await?;
        }
    }
    Ok(())
}

/// When this replica's detection sweep last finished, and its error if it
/// failed. Read by the system doctor, so a detector that stopped is visible.
#[derive(Debug, Clone)]
pub struct SweepRecord {
    pub at: DateTime<Utc>,
    pub error: Option<String>,
}

static LAST_SWEEP: Mutex<Option<SweepRecord>> = Mutex::new(None);

/// Note that a sweep finished.
pub fn record_sweep(error: Option<String>) {
    if let Ok(mut last) = LAST_SWEEP.lock() {
        *last = Some(SweepRecord {
            at: Utc::now(),
            error,
        });
    }
}

/// The last finished sweep on this replica, if one ran.
pub fn last_sweep() -> Option<SweepRecord> {
    LAST_SWEEP.lock().ok().and_then(|last| last.clone())
}
