// Observer domain types — online scoring of production sessions (EVE-878).
//
// Decision: Observers, their judge configuration, and the hosted trace-score
// lifecycle are product management/reporting aggregates. They watch completed
// turns from the outside and never participate in a turn, so they moved here
// from `everruns-core`. Matching/scoring workers live in
// `crates/server/src/domains/observers/`.
//
// Design Decision: Observers watch real production traffic instead of creating
// synthetic sessions (that is what Evals do). An Observer holds match rules
// (which sessions to score) and scorer configs (how to score them). Scoring is
// asynchronous: matching enqueues pending TraceScore rows, background workers
// claim and complete them.
//
// Design Decision: Scores are derived data with their own lifecycle and live in
// their own table; they are never appended to the immutable session event log.
//
// See knowledge/evaluation/online-evals.md for full specification.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domains::evals::record::Scorer;
use everruns_contracts::typed_id::{AgentId, HarnessId, ObserverId, SessionId, TraceScoreId};

use utoipa::ToSchema;

// ============================================
// Observer status
// ============================================

/// Observer lifecycle status. `paused` keeps configuration but stops matching.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ObserverStatus {
    Active,
    Paused,
    Archived,
    Deleted,
}

impl std::fmt::Display for ObserverStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObserverStatus::Active => write!(f, "active"),
            ObserverStatus::Paused => write!(f, "paused"),
            ObserverStatus::Archived => write!(f, "archived"),
            ObserverStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for ObserverStatus {
    fn from(s: &str) -> Self {
        match s {
            "paused" => ObserverStatus::Paused,
            "archived" => ObserverStatus::Archived,
            "deleted" => ObserverStatus::Deleted,
            _ => ObserverStatus::Active,
        }
    }
}

// ============================================
// Match rules
// ============================================

/// Predicates selecting which production sessions an observer scores.
/// All present predicates must match (AND); within a list, any entry matches (OR).
/// An empty match block matches all org traffic. Sessions tagged `eval` are
/// always excluded so synthetic eval-run sessions are never scored.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct ObserverMatch {
    /// Match sessions running any of these agents.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<String>>)]
    pub agent_ids: Option<Vec<AgentId>>,
    /// Match sessions on any of these harnesses.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<String>>)]
    pub harness_ids: Option<Vec<HarnessId>>,
    /// Match sessions carrying any of these tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_tags: Option<Vec<String>>,
}

impl ObserverMatch {
    /// Evaluate predicates against session attributes.
    pub fn matches(
        &self,
        agent_id: Option<AgentId>,
        harness_id: Option<HarnessId>,
        session_tags: &[String],
    ) -> bool {
        if let Some(agent_ids) = &self.agent_ids {
            let Some(agent_id) = agent_id else {
                return false;
            };
            if !agent_ids.contains(&agent_id) {
                return false;
            }
        }
        if let Some(harness_ids) = &self.harness_ids {
            let Some(harness_id) = harness_id else {
                return false;
            };
            if !harness_ids.contains(&harness_id) {
                return false;
            }
        }
        if let Some(tags) = &self.session_tags
            && !tags.iter().any(|t| session_tags.contains(t))
        {
            return false;
        }
        true
    }
}

// ============================================
// Scorer config
// ============================================

/// What slice of the trace a scorer grades.
///
/// - `turn`: one exchange, scored when the turn completes.
/// - `session`: the whole conversation, scored once the session has been idle
///   for a while ([`SESSION_SETTLE_SECONDS`]); a later idle re-scores it, so
///   each session has one current score.
/// - `tool`: one tool call and its result, scored when the call completes.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ObserverScope {
    #[default]
    Turn,
    Session,
    Tool,
}

impl ObserverScope {
    pub fn as_str(self) -> &'static str {
        match self {
            ObserverScope::Turn => "turn",
            ObserverScope::Session => "session",
            ObserverScope::Tool => "tool",
        }
    }
}

impl std::fmt::Display for ObserverScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ObserverScope {
    fn from(s: &str) -> Self {
        match s {
            "session" => ObserverScope::Session,
            "tool" => ObserverScope::Tool,
            _ => ObserverScope::Turn,
        }
    }
}

/// How long a session stays idle before its session-scope score runs. A
/// person who answers within this time moves it back, so a conversation is
/// judged once it has settled, not after every reply.
pub const SESSION_SETTLE_SECONDS: i64 = 600;

/// The `turn_id` key of a session-scope score: one per session.
pub const SESSION_SCORE_KEY: &str = "session";

/// Default value at/above which an LLM-judge score is considered a pass.
fn default_pass_threshold() -> f64 {
    0.5
}

/// LLM-as-judge scoring configuration. The judge grades the scoped trace
/// slice against `rubric` and returns a 0.0–1.0 value, an optional
/// categorical label, and free-text reasoning.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LlmJudgeConfig {
    /// Grading rubric shown to the judge model. Should describe what a high
    /// vs. low score means.
    pub rubric: String,
    /// Org model to judge with. When `None`, the org's default model is used.
    /// Judge calls go through the org's own providers and are billed to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub model_id: Option<everruns_contracts::typed_id::ModelId>,
    /// Score value at/above which `pass` is true.
    #[serde(default = "default_pass_threshold")]
    pub pass_threshold: f64,
}

/// How a scorer grades a trace slice: a deterministic `rule` (reusing the
/// eval scorer vocabulary) or an `llm_judge`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum ScorerMethod {
    /// Deterministic rule. `file_contains` is rejected for observers (session
    /// filesystems are not part of the observable trace contract).
    Rule { rule: Scorer },
    /// LLM-as-judge.
    LlmJudge(LlmJudgeConfig),
}

/// One scorer inside an observer. `key` names the score series in listings
/// and future dashboards; `scope` selects the trace slice; `method` is how
/// it grades.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ObserverScorerConfig {
    /// Stable name within the observer (score series name).
    pub key: String,
    /// Trace slice this scorer grades.
    #[serde(default)]
    pub scope: ObserverScope,
    /// Tool scope only: grade calls to these tools. Empty or absent grades
    /// every tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    /// How this scorer grades (rule or llm_judge).
    #[serde(flatten)]
    pub method: ScorerMethod,
}

impl ObserverScorerConfig {
    /// Whether a tool-scope scorer grades a call to `tool`.
    pub fn grades_tool(&self, tool: &str) -> bool {
        self.tools
            .as_ref()
            .is_none_or(|tools| tools.is_empty() || tools.iter().any(|t| t == tool))
    }
}

// ============================================
// Alerts and reviews
// ============================================

fn default_alert_window_hours() -> u32 {
    24
}

fn default_alert_min_scored() -> u32 {
    20
}

/// Open a health issue, and tell the people who own the watched agents, when
/// an observer's pass rate falls under `min_pass_rate` over the last
/// `window_hours`. Needs `min_scored` scores in the window so a handful of
/// samples cannot raise it. The issue resolves itself once the rate recovers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct ObserverAlert {
    /// Pass rate (0.0–1.0) under which the alert opens.
    #[schema(example = 0.8)]
    pub min_pass_rate: f64,
    /// Window the rate is measured over, in hours (1–168). Defaults to 24.
    #[serde(default = "default_alert_window_hours")]
    #[schema(example = 24)]
    pub window_hours: u32,
    /// Scores the window needs before the alert can open. Defaults to 20.
    #[serde(default = "default_alert_min_scored")]
    #[schema(example = 20)]
    pub min_scored: u32,
}

/// A person's verdict on one score: whether the grader got it right. Reviews
/// calibrate graders, and a disagreed score is a candidate eval case.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct ScoreReview {
    /// True when the person agrees with the grader's pass or fail.
    pub agrees: bool,
    /// Optional note on why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Who reviewed (user id), set by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_by: Option<String>,
    /// When, set by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<DateTime<Utc>>,
}

// ============================================
// Observer entity
// ============================================

/// An observer: online scoring config over production sessions.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Observer {
    /// External identifier (observer_<32-hex>). Shown as "id" in API.
    #[serde(rename = "id")]
    #[schema(value_type = String, example = "observer_01933b5a000070008000000000000001")]
    pub public_id: ObserverId,
    /// Internal UUID primary key. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Organization ID. Internal only.
    #[serde(skip, default)]
    pub org_id: i64,
    /// Display name.
    #[schema(example = "Support answer quality")]
    pub name: String,
    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Grades support replies for grounded answers.")]
    pub description: Option<String>,
    /// Which sessions to score.
    #[serde(rename = "match", default)]
    pub match_config: ObserverMatch,
    /// Fraction of matching turns to score (0.0–1.0), applied after match.
    #[schema(example = 0.1)]
    pub sampling_rate: f64,
    /// Scoring rules.
    pub scorers: Vec<ObserverScorerConfig>,
    /// Pass-rate alert, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert: Option<ObserverAlert>,
    /// Lifecycle status.
    pub status: ObserverStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
}

// ============================================
// Trace scores
// ============================================

/// Lifecycle of one trace score. `pending` rows double as the scoring queue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum TraceScoreStatus {
    Pending,
    Scoring,
    Completed,
    Errored,
    Skipped,
}

impl std::fmt::Display for TraceScoreStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TraceScoreStatus::Pending => write!(f, "pending"),
            TraceScoreStatus::Scoring => write!(f, "scoring"),
            TraceScoreStatus::Completed => write!(f, "completed"),
            TraceScoreStatus::Errored => write!(f, "errored"),
            TraceScoreStatus::Skipped => write!(f, "skipped"),
        }
    }
}

impl From<&str> for TraceScoreStatus {
    fn from(s: &str) -> Self {
        match s {
            "scoring" => TraceScoreStatus::Scoring,
            "completed" => TraceScoreStatus::Completed,
            "errored" => TraceScoreStatus::Errored,
            "skipped" => TraceScoreStatus::Skipped,
            _ => TraceScoreStatus::Pending,
        }
    }
}

/// One score produced by an observer scorer for one trace slice. Linked back
/// to the exact session/turn it graded; agent/harness identifiers are
/// denormalized at scoring time for aggregation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TraceScore {
    /// External identifier (score_<32-hex>). Shown as "id" in API.
    #[serde(rename = "id")]
    #[schema(value_type = String, example = "score_01933b5a000070008000000000000001")]
    pub public_id: TraceScoreId,
    /// Internal UUID primary key. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Organization ID. Internal only.
    #[serde(skip, default)]
    pub org_id: i64,
    /// Observer that produced this score.
    #[schema(value_type = String, example = "observer_01933b5a000070008000000000000001")]
    pub observer_id: ObserverId,
    /// Scorer key within the observer.
    #[schema(example = "grounded")]
    pub scorer_key: String,
    /// Session this score grades.
    #[schema(value_type = String, example = "session_01933b5a000070008000000000000001")]
    pub session_id: SessionId,
    /// Trace slice this score grades.
    #[serde(default)]
    pub scope: ObserverScope,
    /// What was graded: the turn id (turn scope), `session` (session scope)
    /// or the tool call id (tool scope).
    pub turn_id: String,
    /// Agent active in the session at scoring time.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub agent_id: Option<AgentId>,
    /// Harness of the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub harness_id: Option<HarnessId>,
    pub status: TraceScoreStatus,
    /// Whether the scorer passed (set when completed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pass: Option<bool>,
    /// Score value 0.0–1.0 (set when completed).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.85)]
    pub value: Option<f64>,
    /// Optional categorical label from an LLM judge (e.g. `missing_source`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Human-readable explanation (set when completed). For LLM judges this is
    /// the judge's reasoning — retained as the raw material for the Phase 2
    /// improvement loop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Judge LLM input tokens (llm_judge scores only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_input_tokens: Option<u64>,
    /// Judge LLM output tokens (llm_judge scores only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_output_tokens: Option<u64>,
    /// Judge call cost in USD when the provider reports it (llm_judge only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_cost_usd: Option<f64>,
    /// Error details if errored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// A person's review of this score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ScoreReview>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================
// Live quality summary and backfill
// ============================================

/// Completed scores in one group of a live-quality summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct QualityGroup {
    /// An agent id, an observer id or a day (`YYYY-MM-DD`), by list.
    #[schema(example = "agent_01933b5a000070008000000000000001")]
    pub key: String,
    pub scored: u64,
    pub passed: u64,
    /// `passed / scored`; absent when nothing was scored.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.92)]
    pub pass_rate: Option<f64>,
    /// Distinct sessions the scores read.
    pub sessions: u64,
    /// Daily pass rates, oldest first (agent groups only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trend: Vec<f64>,
}

/// How production traffic scored over the last `days`: in total, by agent,
/// by observer and by day.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct QualitySummary {
    /// Start of the window.
    pub since: DateTime<Utc>,
    #[schema(example = 7)]
    pub days: u32,
    pub scored: u64,
    pub passed: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.92)]
    pub pass_rate: Option<f64>,
    pub sessions: u64,
    pub by_agent: Vec<QualityGroup>,
    pub by_observer: Vec<QualityGroup>,
    pub by_day: Vec<QualityGroup>,
}

/// What a backfill read and queued.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct BackfillResult {
    /// Past turn, tool and idle events read.
    pub events_read: u64,
    /// Scores queued. Turns and tool calls already scored are not queued
    /// again; a conversation's score is refreshed.
    pub enqueued: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_match_matches_everything() {
        let m = ObserverMatch::default();
        assert!(m.matches(None, None, &[]));
        assert!(m.matches(Some(AgentId::new()), Some(HarnessId::new()), &["x".into()]));
    }

    #[test]
    fn match_agent_predicate() {
        let agent = AgentId::new();
        let m = ObserverMatch {
            agent_ids: Some(vec![agent]),
            ..Default::default()
        };
        assert!(m.matches(Some(agent), None, &[]));
        assert!(!m.matches(Some(AgentId::new()), None, &[]));
        assert!(!m.matches(None, None, &[]));
    }

    #[test]
    fn match_harness_predicate() {
        let harness = HarnessId::new();
        let m = ObserverMatch {
            harness_ids: Some(vec![harness]),
            ..Default::default()
        };
        assert!(m.matches(None, Some(harness), &[]));
        assert!(!m.matches(None, Some(HarnessId::new()), &[]));
        assert!(!m.matches(None, None, &[]));
    }

    #[test]
    fn match_tags_any_of() {
        let m = ObserverMatch {
            session_tags: Some(vec!["prod".into(), "beta".into()]),
            ..Default::default()
        };
        assert!(m.matches(None, None, &["beta".into()]));
        assert!(!m.matches(None, None, &["other".into()]));
        assert!(!m.matches(None, None, &[]));
    }

    #[test]
    fn match_predicates_are_anded() {
        let agent = AgentId::new();
        let m = ObserverMatch {
            agent_ids: Some(vec![agent]),
            session_tags: Some(vec!["prod".into()]),
            ..Default::default()
        };
        assert!(m.matches(Some(agent), None, &["prod".into()]));
        assert!(!m.matches(Some(agent), None, &[]));
        assert!(!m.matches(None, None, &["prod".into()]));
    }

    #[test]
    fn scorer_config_llm_judge_serde() {
        let json = serde_json::json!({
            "key": "completeness",
            "scope": "turn",
            "method": "llm_judge",
            "rubric": "Score 1 if the answer fully addresses the question."
        });
        let config: ObserverScorerConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.key, "completeness");
        match config.method {
            ScorerMethod::LlmJudge(j) => {
                assert!(j.model_id.is_none());
                assert_eq!(j.pass_threshold, 0.5);
                assert!(j.rubric.contains("fully addresses"));
            }
            _ => panic!("expected llm_judge"),
        }
    }

    #[test]
    fn observer_serde_skips_internal_fields() {
        let observer = Observer {
            public_id: ObserverId::from_uuid(Uuid::nil()),
            internal_id: Uuid::nil(),
            org_id: 1,
            name: "test".into(),
            description: None,
            match_config: ObserverMatch::default(),
            sampling_rate: 0.1,
            scorers: vec![],
            alert: None,
            status: ObserverStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
        };
        let json = serde_json::to_value(&observer).unwrap();
        assert!(json.get("id").is_some());
        assert!(json.get("match").is_some());
        assert!(json.get("internal_id").is_none());
        assert!(json.get("org_id").is_none());
        assert_eq!(json["sampling_rate"], 0.1);
    }
}
