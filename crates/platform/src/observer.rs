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

use crate::eval::Scorer;
use everruns_provider::typed_id::{
    AgentId, AgentVersionId, HarnessId, ObserverId, SessionId, TraceScoreId,
};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

// ============================================
// Observer status
// ============================================

/// Observer lifecycle status. `paused` keeps configuration but stops matching.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ObserverMatch {
    /// Match sessions running any of these agents.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Vec<String>>))]
    pub agent_ids: Option<Vec<AgentId>>,
    /// Match sessions on any of these harnesses.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Vec<String>>))]
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

/// What slice of the trace a scorer grades. Phase 1 implements `turn` only;
/// `session` and `tool` scopes are specced in knowledge/evaluation/online-evals.md.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ObserverScope {
    #[default]
    Turn,
}

impl std::fmt::Display for ObserverScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObserverScope::Turn => write!(f, "turn"),
        }
    }
}

/// Default value at/above which an LLM-judge score is considered a pass.
fn default_pass_threshold() -> f64 {
    0.5
}

/// LLM-as-judge scoring configuration. The judge grades the scoped trace
/// slice against `rubric` and returns a 0.0–1.0 value, an optional
/// categorical label, and free-text reasoning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmJudgeConfig {
    /// Grading rubric shown to the judge model. Should describe what a high
    /// vs. low score means.
    pub rubric: String,
    /// Org model to judge with. When `None`, the org's default model is used.
    /// Judge calls go through the org's own providers and are billed to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub model_id: Option<everruns_provider::typed_id::ModelId>,
    /// Score value at/above which `pass` is true.
    #[serde(default = "default_pass_threshold")]
    pub pass_threshold: f64,
}

/// How a scorer grades a trace slice: a deterministic `rule` (reusing the
/// eval scorer vocabulary) or an `llm_judge`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ObserverScorerConfig {
    /// Stable name within the observer (score series name).
    pub key: String,
    /// Trace slice this scorer grades.
    #[serde(default)]
    pub scope: ObserverScope,
    /// How this scorer grades (rule or llm_judge).
    #[serde(flatten)]
    pub method: ScorerMethod,
}

// ============================================
// Observer entity
// ============================================

/// An observer: online scoring config over production sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Observer {
    /// External identifier (observer_<32-hex>). Shown as "id" in API.
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "observer_01933b5a000070008000000000000001"))]
    pub public_id: ObserverId,
    /// Internal UUID primary key. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Organization ID. Internal only.
    #[serde(skip, default)]
    pub org_id: i64,
    /// Display name.
    pub name: String,
    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Which sessions to score.
    #[serde(rename = "match", default)]
    pub match_config: ObserverMatch,
    /// Fraction of matching turns to score (0.0–1.0), applied after match.
    pub sampling_rate: f64,
    /// Scoring rules.
    pub scorers: Vec<ObserverScorerConfig>,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TraceScore {
    /// External identifier (score_<32-hex>). Shown as "id" in API.
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "score_01933b5a000070008000000000000001"))]
    pub public_id: TraceScoreId,
    /// Internal UUID primary key. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Organization ID. Internal only.
    #[serde(skip, default)]
    pub org_id: i64,
    /// Observer that produced this score.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub observer_id: ObserverId,
    /// Scorer key within the observer.
    pub scorer_key: String,
    /// Session this score grades.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub session_id: SessionId,
    /// Turn this score grades (turn scope).
    pub turn_id: String,
    /// Agent active in the session at scoring time.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub agent_id: Option<AgentId>,
    /// Agent version active in the session at scoring time.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub agent_version_id: Option<AgentVersionId>,
    /// Harness of the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub harness_id: Option<HarnessId>,
    pub status: TraceScoreStatus,
    /// Whether the scorer passed (set when completed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pass: Option<bool>,
    /// Score value 0.0–1.0 (set when completed).
    #[serde(skip_serializing_if = "Option::is_none")]
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_status_roundtrip() {
        for (s, v) in [
            ("active", ObserverStatus::Active),
            ("paused", ObserverStatus::Paused),
            ("archived", ObserverStatus::Archived),
            ("deleted", ObserverStatus::Deleted),
        ] {
            assert_eq!(ObserverStatus::from(s), v);
            assert_eq!(v.to_string(), s);
        }
        assert_eq!(ObserverStatus::from("unknown"), ObserverStatus::Active);
    }

    #[test]
    fn trace_score_status_roundtrip() {
        for (s, v) in [
            ("pending", TraceScoreStatus::Pending),
            ("scoring", TraceScoreStatus::Scoring),
            ("completed", TraceScoreStatus::Completed),
            ("errored", TraceScoreStatus::Errored),
            ("skipped", TraceScoreStatus::Skipped),
        ] {
            assert_eq!(TraceScoreStatus::from(s), v);
            assert_eq!(v.to_string(), s);
        }
        assert_eq!(TraceScoreStatus::from("unknown"), TraceScoreStatus::Pending);
    }

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
    fn scorer_config_serde_defaults_scope() {
        let json = serde_json::json!({
            "key": "greeting",
            "method": "rule",
            "rule": { "type": "contains", "text": "hello" }
        });
        let config: ObserverScorerConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.scope, ObserverScope::Turn);
        assert_eq!(config.key, "greeting");
        assert!(matches!(config.method, ScorerMethod::Rule { .. }));
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
