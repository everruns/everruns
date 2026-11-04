//! The Inspector (Evals rebuild, detection layer 3;
//! knowledge/operations/background-limits.md, "Detection"): it writes up the
//! findings that matter most. For each open finding it reads the session the
//! finding points at, says what happened, how serious it is and what to do,
//! and files the finding under a cluster of similar ones so ten copies of one
//! problem show as one.
//!
//! Decisions:
//! - What it reads: only findings worth a model call, the classifier findings
//!   (already above their threshold) and runaway schedules. Layer 1 numbers
//!   are cheap and frequent; writing each one up would cost more than it
//!   tells.
//! - It runs on the org's own default model, like observer judges, and its
//!   cost is monitoring spend: the org and agent budgets pay, never an end
//!   user's limits.
//! - Clustering is the Inspector's choice among the org's clusters of the same
//!   kind from the last 30 days, or a new one. It never groups across orgs.
//!   Clusters are leads for a person, never grounds for automatic action.
//! - What is stored is the write-up and the cluster, not the session text it
//!   read. The prompt asks it not to repeat secrets or personal details.
//! - A write-up is made once per finding episode. When the org has no model,
//!   the episode is marked unavailable and not retried; a failed call is
//!   retried after an hour, at most three times.

use super::{background_work, classifiers, fixes};
use crate::domains::budgets::service::BudgetService;
use crate::domains::observers::judge::{CompletionOptions, LlmJudgeClient};
use crate::storage::StorageBackend;
use crate::storage::finding_fixes::NewFindingFix;
use crate::storage::finding_investigations::{
    CompletedInvestigation, FindingClusterRow, InvestigationCandidateRow,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::SessionId;
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

/// The finding kinds the Inspector writes up.
pub const INVESTIGATED: [&str; 4] = [
    classifiers::SUSPECTED_ABUSE,
    classifiers::FOLLOWED_INJECTION,
    classifiers::AGENT_STUCK,
    background_work::SCHEDULE_RUNAWAY,
];

/// Findings of the platform operator's concern: they also appear in the
/// deployment-wide detections list and the system doctor.
pub const PLATFORM_CODES: [&str; 1] = [classifiers::SUSPECTED_ABUSE];

/// Findings written up per sweep and replica.
const PER_SWEEP: i64 = 10;
/// A failed write-up is tried again after this long, this many times.
const RETRY: Duration = Duration::from_secs(3600);
const MAX_ATTEMPTS: i32 = 3;
/// Clusters offered for filing: the org's clusters of the kind seen in this
/// many days, at most this many.
const CLUSTER_DAYS: i32 = 30;
const CLUSTER_CHOICES: i64 = 12;
/// What it reads of the session.
const MESSAGES: i64 = 12;
const MESSAGE_CHARS: usize = 1200;
const TOOL_WINDOW: i64 = 60;
/// Bounds on what it writes.
const WHAT_HAPPENED_CHARS: usize = 1200;
const ACTION_CHARS: usize = 400;
const TITLE_CHARS: usize = 80;
/// What it reads of the agent's instructions, for a fix.
const INSTRUCTION_CHARS: usize = 6000;

const SWEEP_INTERVAL: Duration = Duration::from_secs(15 * 60);
pub const INSPECTOR_PURPOSE: &str = "finding_investigation";

/// One write-up and what it cost.
#[derive(Debug, Clone)]
pub struct InspectorReply {
    pub text: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
}

/// Writes findings up. `Ok(None)` when the org has no model it can use.
#[async_trait]
pub trait InspectorModel: Send + Sync {
    async fn write_up(
        &self,
        org_id: i64,
        system: String,
        prompt: String,
    ) -> anyhow::Result<Option<InspectorReply>>;
}

/// The production Inspector: the org's default model.
pub struct LlmInspector {
    client: Arc<LlmJudgeClient>,
}

impl LlmInspector {
    pub fn new(client: Arc<LlmJudgeClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl InspectorModel for LlmInspector {
    async fn write_up(
        &self,
        org_id: i64,
        system: String,
        prompt: String,
    ) -> anyhow::Result<Option<InspectorReply>> {
        let Some(completion) = self
            .client
            .complete(
                org_id,
                None,
                system,
                prompt,
                CompletionOptions {
                    temperature: 0.2,
                    max_tokens: 900,
                    purpose: INSPECTOR_PURPOSE,
                },
            )
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(InspectorReply {
            text: completion.text,
            model: completion.model,
            input_tokens: completion.input_tokens.unwrap_or(0),
            output_tokens: completion.output_tokens.unwrap_or(0),
            cost_usd: completion.cost_usd,
        }))
    }
}

/// What a finding is, in words the Inspector reads.
fn kind_label(code: &str) -> &'static str {
    match code {
        classifiers::SUSPECTED_ABUSE => {
            "A decision model judged this session likely to be abuse of the platform or of \
             other people (spam, scraping at scale, credential attacks, crypto mining, \
             phishing, malware or scams)."
        }
        classifiers::FOLLOWED_INJECTION => {
            "A decision model judged that the agent acted on instructions from a tool result \
             or fetched content rather than from the person it works for."
        }
        classifiers::AGENT_STUCK => {
            "A decision model judged that the agent is going in circles without progress."
        }
        background_work::SCHEDULE_RUNAWAY => {
            "A schedule keeps firing and producing the same result or growing cost without \
             a person in the loop."
        }
        _ => "A detector flagged this session.",
    }
}

pub const SYSTEM_PROMPT: &str = "You are the Inspector, who reviews findings that automatic \
detectors raised about an AI agent's work, for the people who run that agent. Read the \
evidence and write a short, plain account. Say what actually happened and why it matters, in \
two to four sentences a busy operator understands without opening the session. Say how \
serious it is: high when people, money or the platform are being harmed or will be soon, \
medium when work is wasted or going wrong, low when the detector was probably wrong. Suggest \
one concrete next step. Do not repeat secrets, credentials or personal details from the \
session; describe them instead. If the detector looks wrong, say so plainly.\n\n\
Then file the finding: if one of the listed clusters describes the same underlying problem, \
give its id; otherwise give null and a short title for a new cluster that names the problem, \
not this one session.\n\n\
When the agent's instructions are shown and a short addition to them would have prevented \
this, propose it: one line saying what the change does, and the exact text to add at the end \
of the instructions, written as an instruction to the agent. Propose nothing that loosens a \
safety rule, grants more access, or hides the problem from people; give null when the problem \
is not the agent's instructions.\n\n\
Answer with one JSON object and nothing else: {\"what_happened\": string, \"severity\": \
\"high\" | \"medium\" | \"low\", \"suggested_action\": string, \"cluster\": string or null, \
\"cluster_title\": string, \"instruction_change\": {\"summary\": string, \"add\": string} \
or null}.";

/// The prompt for one finding.
pub fn prompt(
    code: &str,
    evidence: Option<&serde_json::Value>,
    messages: &[(String, String)],
    tools: &[serde_json::Value],
    clusters: &[FindingClusterRow],
    instructions: Option<&str>,
) -> String {
    let mut out = format!("Finding: {}\n", kind_label(code));
    if let Some(evidence) = evidence.filter(|e| !e.is_null()) {
        out.push_str(&format!("Detector evidence: {evidence}\n"));
    }
    out.push_str("\nRecent conversation, oldest first:\n");
    if messages.is_empty() {
        out.push_str("(no messages)\n");
    }
    for (from, text) in messages {
        out.push_str(&format!("{from}: {}\n", cut(text, MESSAGE_CHARS)));
    }
    out.push_str("\nRecent tool calls (tool, target, calls, failed):\n");
    if tools.is_empty() {
        out.push_str("(none)\n");
    }
    for tool in tools {
        out.push_str(&format!("{tool}\n"));
    }
    out.push_str("\nClusters of this kind in this organization:\n");
    if clusters.is_empty() {
        out.push_str("(none yet)\n");
    }
    for (index, cluster) in clusters.iter().enumerate() {
        out.push_str(&format!("C{}: {}\n", index + 1, cluster.title));
    }
    if let Some(instructions) = instructions {
        out.push_str("\nThe agent's instructions:\n");
        out.push_str(&cut(instructions, INSTRUCTION_CHARS));
        out.push('\n');
    }
    out
}

/// The Inspector's answer, checked and trimmed.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteUp {
    pub what_happened: String,
    pub severity: String,
    pub suggested_action: String,
    /// Index into the offered clusters, when it chose one.
    pub cluster: Option<usize>,
    pub cluster_title: String,
    /// A proposed addition to the agent's instructions: (summary, text).
    pub fix: Option<(String, String)>,
}

#[derive(Deserialize)]
struct RawFix {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    add: String,
}

#[derive(Deserialize)]
struct RawWriteUp {
    what_happened: String,
    severity: String,
    #[serde(default)]
    suggested_action: String,
    #[serde(default)]
    cluster: Option<String>,
    #[serde(default)]
    cluster_title: Option<String>,
    #[serde(default)]
    instruction_change: Option<RawFix>,
}

/// Parse the Inspector's answer. `None` when it is not the asked-for JSON.
pub fn parse_write_up(text: &str, clusters: usize) -> Option<WriteUp> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let raw: RawWriteUp = serde_json::from_str(text.get(start..=end)?).ok()?;
    let what_happened = cut(raw.what_happened.trim(), WHAT_HAPPENED_CHARS);
    if what_happened.is_empty() {
        return None;
    }
    let severity = raw.severity.trim().to_ascii_lowercase();
    if !matches!(severity.as_str(), "high" | "medium" | "low") {
        return None;
    }
    let cluster = raw
        .cluster
        .as_deref()
        .map(|c| c.trim().trim_start_matches(['C', 'c']))
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| (1..=clusters).contains(n))
        .map(|n| n - 1);
    let title = raw
        .cluster_title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| cut(t, TITLE_CHARS))
        .unwrap_or_else(|| cut(&what_happened, TITLE_CHARS));
    // A fix over its bounds is dropped, not cut: half an instruction is
    // worse than none.
    let fix = raw.instruction_change.and_then(|f| {
        let add = f.add.trim();
        let summary = f.summary.trim();
        (!add.is_empty() && add.chars().count() <= fixes::ADDITION_CHARS).then(|| {
            let summary = if summary.is_empty() { add } else { summary };
            (cut(summary, fixes::SUMMARY_CHARS - 1), add.to_string())
        })
    });
    Some(WriteUp {
        what_happened,
        severity,
        suggested_action: cut(raw.suggested_action.trim(), ACTION_CHARS),
        cluster,
        cluster_title: title,
        fix,
    })
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{}…", head.trim_end())
}

/// What one sweep did on this replica, for the system doctor.
#[derive(Debug, Clone, Default)]
pub struct InspectorSweep {
    pub at: Option<DateTime<Utc>>,
    pub written: usize,
    pub unavailable: usize,
    pub failed: usize,
    pub last_error: Option<String>,
    /// Open findings of the platform's concern across the deployment.
    pub open_platform: i64,
}

static LAST_SWEEP: Mutex<Option<InspectorSweep>> = Mutex::new(None);

/// The last finished Inspector sweep on this replica, if one ran.
pub fn last_sweep() -> Option<InspectorSweep> {
    LAST_SWEEP.lock().ok().and_then(|last| last.clone())
}

/// One pass: write up the open findings that have no write-up yet.
pub async fn sweep(
    db: &Arc<StorageBackend>,
    model: &dyn InspectorModel,
) -> anyhow::Result<InspectorSweep> {
    let mut report = InspectorSweep::default();
    let retry = RETRY.as_secs() as i64;
    let candidates = db
        .findings_to_investigate(&INVESTIGATED, retry, MAX_ATTEMPTS, PER_SWEEP)
        .await?;
    for candidate in candidates {
        if !db
            .claim_finding_investigation(
                candidate.health_issue_id,
                candidate.episode_id,
                candidate.org_id,
                retry,
            )
            .await?
        {
            continue;
        }
        if let Err(error) = investigate(db, model, &candidate, &mut report).await {
            report.failed += 1;
            report.last_error = Some(error.to_string());
            db.fail_finding_investigation(
                candidate.health_issue_id,
                candidate.episode_id,
                "failed",
                &error.to_string(),
            )
            .await?;
        }
    }
    report.open_platform = db.count_platform_detections(&PLATFORM_CODES).await?;
    report.at = Some(Utc::now());
    Ok(report)
}

async fn investigate(
    db: &Arc<StorageBackend>,
    model: &dyn InspectorModel,
    candidate: &InvestigationCandidateRow,
    report: &mut InspectorSweep,
) -> anyhow::Result<()> {
    let (messages, tools) = match candidate.session_id {
        Some(session) => {
            let messages = db
                .classifier_messages(session, MESSAGES)
                .await?
                .into_iter()
                .map(|m| (m.role, m.text))
                .collect();
            let tools = db
                .classifier_tools(session, TOOL_WINDOW)
                .await?
                .into_iter()
                .map(|t| {
                    serde_json::json!([
                        t.name,
                        t.target.as_deref().map(|target| cut(target, 200)),
                        t.calls,
                        t.failed
                    ])
                })
                .collect();
            (messages, tools)
        }
        None => (Vec::new(), Vec::new()),
    };
    let clusters = db
        .recent_finding_clusters(
            candidate.org_id,
            &candidate.code,
            CLUSTER_DAYS,
            CLUSTER_CHOICES,
        )
        .await?;
    // The agent's instructions, when the finding may get a fix and the org
    // has not turned suggested fixes off.
    let fixes_wanted = fixes::FIXABLE.contains(&candidate.code.as_str())
        && super::self_healing::policy(db, candidate.org_id)
            .await
            .agent_fixes
            != super::self_healing::SelfHealingMode::Off;
    let agent = match candidate.agent_id {
        Some(agent_id) if fixes_wanted => db
            .get_agent(
                candidate.org_id,
                everruns_contracts::typed_id::AgentId::from_uuid(agent_id),
            )
            .await?
            .filter(|a| a.status != "archived"),
        _ => None,
    };
    let text = prompt(
        &candidate.code,
        candidate.evidence.as_ref(),
        &messages,
        &tools,
        &clusters,
        agent.as_ref().map(|a| a.system_prompt.as_str()),
    );
    let Some(reply) = model
        .write_up(candidate.org_id, SYSTEM_PROMPT.to_string(), text)
        .await?
    else {
        report.unavailable += 1;
        db.fail_finding_investigation(
            candidate.health_issue_id,
            candidate.episode_id,
            "unavailable",
            "The organization has no default model the Inspector can use",
        )
        .await?;
        return Ok(());
    };
    if let Some(session) = candidate.session_id {
        BudgetService::new(db.clone())
            .meter_monitoring(
                crate::domains::budgets::service::INSPECTOR_SOURCE,
                SessionId::from_uuid(session),
                // Every call is charged, retries included.
                Uuid::new_v4().to_string(),
                candidate.health_issue_id,
                reply.input_tokens,
                reply.output_tokens,
                reply.cost_usd,
            )
            .await;
    }
    let Some(write_up) = parse_write_up(&reply.text, clusters.len()) else {
        anyhow::bail!("The Inspector's answer was not the expected JSON");
    };
    let cluster_id = db
        .file_finding_cluster(
            candidate.org_id,
            &candidate.code,
            write_up.cluster.map(|i| clusters[i].id),
            &write_up.cluster_title,
        )
        .await?;
    db.complete_finding_investigation(
        candidate.health_issue_id,
        candidate.episode_id,
        CompletedInvestigation {
            what_happened: &write_up.what_happened,
            severity: &write_up.severity,
            suggested_action: &write_up.suggested_action,
            cluster_id,
            model: &reply.model,
            cost_usd: reply.cost_usd,
        },
    )
    .await?;
    if let (Some(agent), Some((summary, addition))) = (&agent, &write_up.fix) {
        db.create_finding_fix(NewFindingFix {
            org_id: candidate.org_id,
            health_issue_id: candidate.health_issue_id,
            episode_id: candidate.episode_id,
            agent_id: agent.id.uuid(),
            summary,
            addition,
            base_instructions_sha256: &fixes::instructions_sha256(&agent.system_prompt),
        })
        .await?;
    }
    report.written += 1;
    Ok(())
}

/// Run [`sweep`] every 15 minutes.
pub fn spawn_sweep(
    db: Arc<StorageBackend>,
    model: Arc<dyn InspectorModel>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let report = match sweep(&db, model.as_ref()).await {
                Ok(report) => report,
                Err(error) => {
                    tracing::warn!(%error, "Inspector sweep failed");
                    InspectorSweep {
                        at: Some(Utc::now()),
                        failed: 1,
                        last_error: Some(error.to_string()),
                        ..Default::default()
                    }
                }
            };
            if let Ok(mut last) = LAST_SWEEP.lock() {
                *last = Some(report);
            }
            let jitter = Uuid::new_v4().as_u128() % 61;
            tokio::time::sleep(SWEEP_INTERVAL + Duration::from_secs(jitter as u64)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster(title: &str) -> FindingClusterRow {
        FindingClusterRow {
            id: Uuid::new_v4(),
            title: title.to_string(),
        }
    }

    #[test]
    fn a_well_formed_answer_parses_and_picks_an_offered_cluster() {
        let text = r#"Here it is: {"what_happened": "The schedule polls a claim that expired.",
            "severity": "High", "suggested_action": "Add a stop rule.",
            "cluster": "C2", "cluster_title": "Expired claim polling"}"#;
        let w = parse_write_up(text, 2).unwrap();
        assert_eq!(w.severity, "high");
        assert_eq!(w.cluster, Some(1));
        assert_eq!(w.cluster_title, "Expired claim polling");
        assert_eq!(w.suggested_action, "Add a stop rule.");
    }

    #[test]
    fn an_unknown_cluster_files_a_new_one_titled_by_the_answer() {
        let text = r#"{"what_happened": "Spam to many addresses.", "severity": "high",
            "suggested_action": "Stop the session.", "cluster": "C9", "cluster_title": ""}"#;
        let w = parse_write_up(text, 2).unwrap();
        assert_eq!(w.cluster, None);
        assert_eq!(w.cluster_title, "Spam to many addresses.");
        let null = r#"{"what_happened": "x", "severity": "low", "cluster": null}"#;
        assert_eq!(parse_write_up(null, 3).unwrap().cluster, None);
    }

    #[test]
    fn answers_without_a_story_or_with_a_made_up_severity_are_rejected() {
        assert!(parse_write_up("no json here", 0).is_none());
        assert!(parse_write_up(r#"{"what_happened": " ", "severity": "high"}"#, 0).is_none());
        assert!(parse_write_up(r#"{"what_happened": "x", "severity": "urgent"}"#, 0).is_none());
    }

    #[test]
    fn long_answers_are_cut_to_their_bounds() {
        let long = "a".repeat(5000);
        let text = format!(
            r#"{{"what_happened": "{long}", "severity": "medium", "suggested_action": "{long}", "cluster_title": "{long}"}}"#
        );
        let w = parse_write_up(&text, 0).unwrap();
        assert!(w.what_happened.chars().count() <= WHAT_HAPPENED_CHARS + 1);
        assert!(w.suggested_action.chars().count() <= ACTION_CHARS + 1);
        assert!(w.cluster_title.chars().count() <= TITLE_CHARS + 1);
    }

    #[test]
    fn the_prompt_names_the_finding_evidence_conversation_and_clusters() {
        let text = prompt(
            classifiers::AGENT_STUCK,
            Some(&serde_json::json!({"probability": 0.97})),
            &[("user".into(), "check the claim".into())],
            &[serde_json::json!(["http_get", "https://x", 31, 0])],
            &[cluster("Expired claim polling"), cluster("Login loop")],
            None,
        );
        assert!(text.contains("going in circles"));
        assert!(text.contains("0.97"));
        assert!(text.contains("user: check the claim"));
        assert!(text.contains("http_get"));
        assert!(text.contains("C1: Expired claim polling\nC2: Login loop"));
    }
}
