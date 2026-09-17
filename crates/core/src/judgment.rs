//! System judgment service.
//!
//! A judgment service answers *typed questions* about state: a probability, a
//! selected option, a graded level. Unlike [`crate::UtilityLlmService`], which
//! returns text a caller has to parse, this returns values the caller can act
//! on, so a malformed answer is not a failure mode a call site has to defend
//! against.
//!
//! Like the utility LLM service this is host-owned: configured once per
//! deployment, never agent- or session-configurable, and never an
//! agent-visible model provider. Capability internals reach it through
//! execution context; the vendor and wire format live in the host.
//!
//! Every question in one request is answered over the same state, in parallel,
//! and cannot see the other answers. Call sites should batch: the cost of an
//! extra question is tokens, the cost of an extra request is a round trip.

use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{AgentLoopError, Result};

/// One typed question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JudgmentQuestion {
    /// Whether a condition holds, answered as the probability of yes.
    Noul {
        /// The yes/no question.
        instructions: String,
        /// What a yes means.
        yes: Option<String>,
        /// What a no means.
        no: Option<String>,
    },
    /// Exactly one option from a defined set.
    Choice {
        /// What to decide.
        instructions: String,
        /// Option name and an optional description of it. At least two.
        options: Vec<(String, Option<String>)>,
    },
    /// A position along ordered levels.
    Score {
        /// What to rate.
        instructions: String,
        /// Ordered level descriptions, lowest first. At least two.
        levels: Vec<String>,
    },
}

impl JudgmentQuestion {
    /// A yes/no question with no explicit criteria.
    pub fn noul(instructions: impl Into<String>) -> Self {
        Self::Noul {
            instructions: instructions.into(),
            yes: None,
            no: None,
        }
    }

    /// A graded question over ordered levels.
    pub fn score<L: Into<String>>(
        instructions: impl Into<String>,
        levels: impl IntoIterator<Item = L>,
    ) -> Self {
        Self::Score {
            instructions: instructions.into(),
            levels: levels.into_iter().map(Into::into).collect(),
        }
    }

    /// The primitive name, for logs and metrics.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Noul { .. } => "noul",
            Self::Choice { .. } => "choice",
            Self::Score { .. } => "score",
        }
    }
}

/// One evaluation: state plus the questions to ask about it.
#[derive(Debug, Clone, Default)]
pub struct JudgmentRequest {
    /// The content being judged: text, or structured data.
    pub state: serde_json::Value,
    /// Questions keyed by caller-chosen ids, in insertion order. Ids are for
    /// the caller's code and are never sent to the model.
    pub questions: Vec<(String, JudgmentQuestion)>,
    /// Free-form request metadata for host-side attribution.
    pub metadata: HashMap<String, String>,
}

impl JudgmentRequest {
    /// Start a request over `state`.
    pub fn new(state: impl Into<serde_json::Value>) -> Self {
        Self {
            state: state.into(),
            questions: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Add a question under `id`.
    pub fn ask(mut self, id: impl Into<String>, question: JudgmentQuestion) -> Self {
        self.questions.push((id.into(), question));
        self
    }

    /// Attach attribution metadata, such as the calling capability.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// How many questions this request carries.
    pub fn len(&self) -> usize {
        self.questions.len()
    }

    /// Whether no question has been added yet.
    pub fn is_empty(&self) -> bool {
        self.questions.is_empty()
    }
}

/// One typed answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JudgmentAnswer {
    /// Probability that the answer is yes, 0..=1.
    ///
    /// A value near 0.5 means yes and no are near-equally likely — not medium
    /// intensity.
    Noul {
        /// Probability of yes.
        probability: f64,
    },
    /// The selected option and the distribution it came from.
    Choice {
        /// Highest-probability option.
        selected: String,
        /// Probability per option.
        probabilities: BTreeMap<String, f64>,
        /// Distribution concentration, 0..=1.
        confidence: f64,
    },
    /// A probability-weighted position across the levels.
    Score {
        /// Weighted position; lands between levels.
        score: f64,
        /// Probability per level index.
        probabilities: BTreeMap<usize, f64>,
        /// Distribution concentration, 0..=1.
        confidence: f64,
    },
}

impl JudgmentAnswer {
    /// The primitive name, for logs and metrics.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Noul { .. } => "noul",
            Self::Choice { .. } => "choice",
            Self::Score { .. } => "score",
        }
    }

    /// Probability of yes, for a noul answer.
    pub fn probability_yes(&self) -> Option<f64> {
        match self {
            Self::Noul { probability } => Some(*probability),
            _ => None,
        }
    }

    /// Distribution concentration, for the two primitives that report it.
    pub fn confidence(&self) -> Option<f64> {
        match self {
            Self::Noul { .. } => None,
            Self::Choice { confidence, .. } | Self::Score { confidence, .. } => Some(*confidence),
        }
    }

    /// Total probability mass at `level` or above, for a score answer.
    ///
    /// This is the reading that an "any serious hit" rule needs: a bimodal
    /// answer that is probably fine and possibly severe must not average into
    /// fine.
    pub fn probability_at_or_above(&self, level: usize) -> Option<f64> {
        match self {
            Self::Score { probabilities, .. } => Some(
                probabilities
                    .iter()
                    .filter(|(index, _)| **index >= level)
                    .map(|(_, probability)| probability)
                    .sum(),
            ),
            _ => None,
        }
    }
}

/// Token usage for one request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgmentUsage {
    /// Tokens consumed by state and questions.
    pub input_tokens: u64,
    /// Tokens produced by the model.
    pub output_tokens: u64,
}

/// The result of one evaluation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JudgmentOutcome {
    /// The model that answered, as the service resolved it.
    pub model: String,
    /// One answer per question id.
    pub answers: BTreeMap<String, JudgmentAnswer>,
    /// Token usage for the request.
    pub usage: JudgmentUsage,
}

impl JudgmentOutcome {
    /// The answer under `id`, if present.
    pub fn get(&self, id: &str) -> Option<&JudgmentAnswer> {
        self.answers.get(id)
    }
}

/// Host-owned service answering typed questions.
#[async_trait]
pub trait JudgmentService: Send + Sync {
    /// Whether the deployment configured a real service.
    fn is_configured(&self) -> bool;

    /// Answer every question in `request` over its state.
    async fn evaluate(&self, request: JudgmentRequest) -> Result<JudgmentOutcome>;

    /// Implementation name, for logs.
    fn name(&self) -> &'static str {
        "JudgmentService"
    }
}

/// The service a deployment gets when no judgment provider is configured.
#[derive(Debug, Clone, Default)]
pub struct DisabledJudgmentService;

#[async_trait]
impl JudgmentService for DisabledJudgmentService {
    fn is_configured(&self) -> bool {
        false
    }

    async fn evaluate(&self, _request: JudgmentRequest) -> Result<JudgmentOutcome> {
        Err(AgentLoopError::llm("judgment service is disabled"))
    }

    fn name(&self) -> &'static str {
        "DisabledJudgmentService"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disabled_service_reports_itself_and_rejects_requests() {
        let service = DisabledJudgmentService;
        assert!(!service.is_configured());
        let error = service
            .evaluate(JudgmentRequest::new("anything").ask("q", JudgmentQuestion::noul("Yes?")))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "LLM error: judgment service is disabled");
    }

    #[test]
    fn requests_keep_question_order_and_carry_metadata() {
        let request = JudgmentRequest::new(serde_json::json!({"text": "hi"}))
            .ask("second", JudgmentQuestion::noul("b"))
            .ask("first", JudgmentQuestion::score("a", ["low", "high"]))
            .with_metadata("purpose", "guardrails");
        assert_eq!(request.len(), 2);
        assert_eq!(request.questions[0].0, "second");
        assert_eq!(request.questions[1].1.kind(), "score");
        assert_eq!(request.metadata["purpose"], "guardrails");
        assert_eq!(request.state["text"], "hi");
    }

    #[test]
    fn score_tail_mass_is_read_from_the_distribution_not_the_mean() {
        // Probably fine, possibly severe: the mean says 0.6, the tail says 30%.
        let answer = JudgmentAnswer::Score {
            score: 0.6,
            probabilities: BTreeMap::from([(0, 0.7), (1, 0.0), (2, 0.3)]),
            confidence: 0.4,
        };
        assert_eq!(answer.probability_at_or_above(2), Some(0.3));
        assert_eq!(answer.probability_at_or_above(0), Some(1.0));
        assert_eq!(answer.probability_yes(), None);
        assert_eq!(answer.confidence(), Some(0.4));
    }

    #[test]
    fn noul_answers_have_no_separate_confidence() {
        let answer = JudgmentAnswer::Noul { probability: 0.92 };
        assert_eq!(answer.probability_yes(), Some(0.92));
        assert_eq!(answer.confidence(), None);
        assert_eq!(answer.probability_at_or_above(1), None);
    }
}
