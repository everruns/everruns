//! Health issue for live quality (Evals rebuild, step 6): an observer whose
//! pass rate fell under its alert threshold.
//!
//! Decisions:
//! - Only observers with an `alert` are judged; the alert is the person
//!   saying what "bad" means for that observer. There is no global default
//!   threshold: pass rates mean different things per rubric.
//! - The rate is completed scores over the alert's window, and the window
//!   needs `min_scored` of them before it can open, so a handful of samples
//!   cannot page anyone.
//! - The subject is the observer. People told are the authors of the agents
//!   it watches (the org's owners when it watches all traffic or an agent has
//!   no known author).
//! - Notifications carry the rate and counts, never a score's reason: that is
//!   judge text about production content.
//! - Resolves on the next sweep that sees the rate back at or above the
//!   threshold, the window under `min_scored`, or the alert removed.

use super::background_work::{agent_recipients, record, resolve};
use crate::domains::observers::record::{ObserverAlert, ObserverMatch};
use crate::storage::StorageBackend;
use crate::storage::background_work::SubjectHealthIssue;
use chrono::Utc;
use uuid::Uuid;

/// An observer's pass rate is under its alert threshold.
pub const OBSERVER_PASS_RATE_LOW: &str = "observer.pass_rate_low";

/// Whether `code` is this detector's.
pub fn owns(code: &str) -> bool {
    code == OBSERVER_PASS_RATE_LOW
}

/// Whether a window of `scored` scores with `passed` passing breaks `alert`.
pub fn breaches(alert: &ObserverAlert, scored: i64, passed: i64) -> bool {
    scored > 0
        && scored >= alert.min_scored as i64
        && (passed as f64 / scored as f64) < alert.min_pass_rate
}

/// Title and body of this issue in `status`.
pub fn copy_for(status: &str, evidence: &serde_json::Value) -> (String, String) {
    let name = evidence["observer_name"].as_str().unwrap_or("An observer");
    if status == "resolved" {
        return (
            "Live quality back above its alert".into(),
            format!("{name}'s pass rate recovered. Nothing needs doing."),
        );
    }
    let percent = |key: &str| {
        evidence[key]
            .as_f64()
            .map(|v| format!("{:.0}%", v * 100.0))
            .unwrap_or_default()
    };
    (
        "Live quality dropped".into(),
        format!(
            "{name}'s pass rate over the last {} hours is {} ({} of {} scores passed), under its {} alert. Open the observer's failing scores to see what changed.",
            evidence["window_hours"].as_i64().unwrap_or_default(),
            percent("pass_rate"),
            evidence["passed"].as_i64().unwrap_or_default(),
            evidence["scored"].as_i64().unwrap_or_default(),
            percent("min_pass_rate"),
        ),
    )
}

/// One pass over every alerting observer.
pub async fn sweep(db: &StorageBackend) -> anyhow::Result<()> {
    let observers = db.list_alerting_observers().await?;
    let mut breached: Vec<(i64, String)> = Vec::new();
    for observer in &observers {
        let Some(alert) = observer
            .alert
            .clone()
            .and_then(|a| serde_json::from_value::<ObserverAlert>(a).ok())
        else {
            continue;
        };
        let since = Utc::now() - chrono::Duration::hours(alert.window_hours as i64);
        let window = db
            .summarize_trace_scores(
                observer.org_id,
                since,
                crate::storage::ScoreGroup::Total,
                Some(observer.id),
                None,
            )
            .await?
            .into_iter()
            .next();
        let (scored, passed) = window.map(|w| (w.scored, w.passed)).unwrap_or_default();
        if !breaches(&alert, scored, passed) {
            continue;
        }
        let agents = serde_json::from_value::<ObserverMatch>(observer.match_config.clone())
            .ok()
            .and_then(|m| m.agent_ids)
            .unwrap_or_default();
        let only_agent = match agents.as_slice() {
            [agent] => Some(agent.uuid()),
            _ => None,
        };
        let mut recipients: Vec<Uuid> = Vec::new();
        if agents.is_empty() {
            recipients = agent_recipients(db, observer.org_id, None).await;
        }
        for agent in &agents {
            recipients.extend(agent_recipients(db, observer.org_id, Some(agent.uuid())).await);
        }
        recipients.sort();
        recipients.dedup();
        let agent_label = match only_agent {
            Some(agent) => db.agent_label(agent).await?,
            None => None,
        };
        let issue = SubjectHealthIssue {
            org_id: observer.org_id,
            code: OBSERVER_PASS_RATE_LOW,
            subject_id: &observer.public_id,
            agent_id: only_agent,
            session_id: None,
            status: "open",
            evidence: serde_json::json!({
                "observer_name": observer.name,
                "scored": scored,
                "passed": passed,
                "pass_rate": passed as f64 / scored as f64,
                "min_pass_rate": alert.min_pass_rate,
                "window_hours": alert.window_hours,
            }),
            checked_at: Utc::now(),
        };
        record(db, issue, recipients, agent_label, false).await;
        breached.push((observer.org_id, observer.public_id.clone()));
    }
    for row in db
        .open_subject_health_issues(OBSERVER_PASS_RATE_LOW)
        .await?
    {
        if !breached
            .iter()
            .any(|(org, id)| *org == row.org_id && Some(id.as_str()) == row.subject_id.as_deref())
        {
            resolve(db, &row).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(min_pass_rate: f64, min_scored: u32) -> ObserverAlert {
        ObserverAlert {
            min_pass_rate,
            window_hours: 24,
            min_scored,
        }
    }

    #[test]
    fn breaches_needs_enough_scores_under_the_rate() {
        let a = alert(0.8, 20);
        assert!(breaches(&a, 20, 15));
        assert!(!breaches(&a, 20, 16), "exactly at the rate is fine");
        assert!(!breaches(&a, 19, 0), "too few scores");
        assert!(!breaches(&a, 0, 0));
    }

    #[test]
    fn copy_names_rate_and_counts_but_no_score_text() {
        let evidence = serde_json::json!({
            "observer_name": "Support quality",
            "scored": 40,
            "passed": 28,
            "pass_rate": 0.7,
            "min_pass_rate": 0.8,
            "window_hours": 24,
        });
        let (title, body) = copy_for("open", &evidence);
        assert_eq!(title, "Live quality dropped");
        assert!(body.contains("Support quality's pass rate over the last 24 hours is 70%"));
        assert!(body.contains("28 of 40 scores"));
        assert!(body.contains("80% alert"));
        assert_eq!(
            copy_for("resolved", &evidence).0,
            "Live quality back above its alert"
        );
    }
}
