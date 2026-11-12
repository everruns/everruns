//! System decisions.
//!
//! A decisions answers *typed questions* about state: a probability, a
//! selected option, a graded level. Unlike a utility LLM service, which
//! returns text a caller has to parse, this returns values the caller can act
//! on, so a malformed answer is not a failure mode a call site has to defend
//! against.
//!
//! Authentication belongs to the selected provider account; protocol drivers
//! implement this neutral seam. Hosts distinguish deployment-owned guardrails
//! from tenant model bindings and apply the appropriate policy and accounting.
//!
//! Every question in one request is answered over the same state, in parallel,
//! and cannot see the other answers. Call sites should batch: the cost of an
//! extra question is tokens, the cost of an extra request is a round trip.

use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{AgentLoopError, Result};

/// One typed question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DecisionQuestion {
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

impl DecisionQuestion {
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

/// Where a [`DecisionMedia`] item's bytes come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionMediaSource {
    /// The bytes, base64-encoded.
    Data(String),
    /// A URL the vendor fetches itself.
    Uri(String),
}

/// An image, audio clip, video or PDF the model reads alongside the state.
///
/// Only drivers that declare [`media`](crate::decision_driver::DecisionDriverCapabilities::media)
/// accept it (Gemini Decision 1 today); every other driver rejects a request
/// that carries media rather than answering without it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionMedia {
    /// IANA media type, such as `image/png` or `application/pdf`.
    pub mime_type: String,
    /// The content.
    pub source: DecisionMediaSource,
}

impl DecisionMedia {
    /// Media from raw bytes, sent inline.
    ///
    /// ```
    /// use everruns_contracts::decisions::{DecisionMedia, DecisionRequest, DecisionQuestion};
    ///
    /// let photo = DecisionMedia::bytes("image/png", [0x89, b'P', b'N', b'G']);
    /// let request = DecisionRequest::new("A parcel left at a front door.")
    ///     .attach(photo)
    ///     .ask("damaged", DecisionQuestion::noul("Is the parcel damaged?"));
    /// assert_eq!(request.media.len(), 1);
    /// ```
    pub fn bytes(mime_type: impl Into<String>, bytes: impl AsRef<[u8]>) -> Self {
        use base64::Engine as _;
        Self {
            mime_type: mime_type.into(),
            source: DecisionMediaSource::Data(
                base64::engine::general_purpose::STANDARD.encode(bytes),
            ),
        }
    }

    /// Media the vendor fetches from `uri`.
    pub fn uri(mime_type: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            mime_type: mime_type.into(),
            source: DecisionMediaSource::Uri(uri.into()),
        }
    }

    /// Bytes this item adds to a request body.
    pub fn wire_len(&self) -> usize {
        match &self.source {
            DecisionMediaSource::Data(data) | DecisionMediaSource::Uri(data) => data.len(),
        }
    }
}

/// One evaluation: state plus the questions to ask about it.
///
/// `#[non_exhaustive]` because this is an option struct on an alpha surface
/// that grows: `model` was added after the first release and broke every
/// struct-literal construction to do it. Build one with [`Self::new`] and the
/// builder methods, which is what every in-tree call site already does.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct DecisionRequest {
    /// The content being classified: text, or structured data.
    pub state: serde_json::Value,
    /// Questions keyed by caller-chosen ids, in insertion order. Ids are for
    /// the caller's code and are never sent to the model.
    pub questions: Vec<(String, DecisionQuestion)>,
    /// Which model to ask, when the caller wants one in particular.
    ///
    /// `None` leaves the choice to the service, which uses its own default.
    /// A deployment that must pin a model does so by never setting this — the
    /// knob is absent from the config an agent can write, rather than absent
    /// from the type.
    pub model: Option<String>,
    /// Explicit serving provider; model names never select an account.
    pub provider: Option<crate::ProviderKey>,
    /// Free-form request metadata for host-side attribution.
    pub metadata: HashMap<String, String>,
    /// Images, audio, video or PDFs read alongside `state`, in order.
    pub media: Vec<DecisionMedia>,
}

impl DecisionRequest {
    /// Start a request over `state`.
    pub fn new(state: impl Into<serde_json::Value>) -> Self {
        Self {
            state: state.into(),
            model: None,
            provider: None,
            questions: Vec::new(),
            metadata: HashMap::new(),
            media: Vec::new(),
        }
    }

    /// Add an image, audio clip, video or PDF for the model to read.
    pub fn attach(mut self, media: DecisionMedia) -> Self {
        self.media.push(media);
        self
    }

    /// Add a question under `id`.
    pub fn ask(mut self, id: impl Into<String>, question: DecisionQuestion) -> Self {
        self.questions.push((id.into(), question));
        self
    }

    /// Ask a particular model rather than the service's default.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Select a serving provider explicitly, leaving the model name opaque.
    pub fn on(mut self, model: crate::ModelSpec) -> Self {
        self.provider = Some(model.provider);
        self.model = Some(model.model);
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
pub enum DecisionAnswer {
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

impl DecisionAnswer {
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

/// One-hot answers for drivers whose vendor returns a label, not a
/// distribution. Pair them with `DecisionOutcome::calibrated = false`.
impl DecisionAnswer {
    /// A yes/no label: probability 1 for yes, 0 for no.
    pub fn noul_label(yes: bool) -> Self {
        Self::Noul {
            probability: if yes { 1.0 } else { 0.0 },
        }
    }

    /// A selected option out of `options`, with all mass on the selection.
    pub fn choice_label<'a>(
        selected: impl Into<String>,
        options: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let selected = selected.into();
        let probabilities = options
            .into_iter()
            .map(|option| {
                let mass = if option == selected { 1.0 } else { 0.0 };
                (option.to_string(), mass)
            })
            .collect();
        Self::Choice {
            selected,
            probabilities,
            confidence: 1.0,
        }
    }

    /// A level index out of `level_count`, with all mass on that level.
    pub fn score_label(level: usize, level_count: usize) -> Self {
        Self::Score {
            score: level as f64,
            probabilities: (0..level_count)
                .map(|index| (index, if index == level { 1.0 } else { 0.0 }))
                .collect(),
            confidence: 1.0,
        }
    }
}

/// Token usage for one request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionUsage {
    /// Tokens consumed by state and questions.
    pub input_tokens: u64,
    /// Tokens produced by the model.
    pub output_tokens: u64,
}

/// Authoritative upstream attribution; absent cost means unknown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DecisionAttribution {
    pub request_id: Option<String>,
    pub serving_provider: Option<String>,
    pub cost_usd: Option<f64>,
}

/// The result of one evaluation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DecisionOutcome {
    /// The model that answered, as the service resolved it.
    pub model: String,
    /// One answer per question id.
    pub answers: BTreeMap<String, DecisionAnswer>,
    /// Token usage for the request.
    pub usage: DecisionUsage,
    /// Whether the numbers are a measured distribution.
    ///
    /// `true` when the answering vendor returned probabilities (TypeSafe's
    /// System One models do). `false` when it returned only a label: the
    /// driver then encodes that label as a one-hot distribution, so every
    /// accessor still works and a threshold still reads as "did the model
    /// pick this", but the numbers are not evidence of how sure it was.
    /// Drivers never invent intermediate probabilities to fill the gap.
    /// Defaults to `false`, the claim that cannot overstate.
    pub calibrated: bool,
    pub attribution: Option<DecisionAttribution>,
}

impl DecisionOutcome {
    /// Whether the numbers are a measured distribution; see [`Self::calibrated`].
    pub fn is_calibrated(&self) -> bool {
        self.calibrated
    }

    /// The answer under `id`, if present.
    pub fn get(&self, id: &str) -> Option<&DecisionAnswer> {
        self.answers.get(id)
    }
}

/// Host-owned service answering typed questions.
#[async_trait]
pub trait DecisionsService: Send + Sync {
    /// Whether the deployment configured a real service.
    fn is_configured(&self) -> bool;

    /// Answer every question in `request` over its state.
    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionOutcome>;

    /// Implementation name, for logs.
    fn name(&self) -> &'static str {
        "DecisionsService"
    }
}

/// The service a deployment gets when no decisions is configured.
#[derive(Debug, Clone, Default)]
pub struct DisabledDecisionsService;

#[async_trait]
impl DecisionsService for DisabledDecisionsService {
    fn is_configured(&self) -> bool {
        false
    }

    async fn evaluate(&self, _request: DecisionRequest) -> Result<DecisionOutcome> {
        Err(AgentLoopError::llm(
            "decisions is disabled (no decision driver configured: set \
             UTILITY_TYPESAFE_API_KEY, or UTILITY_DECISION_DRIVER=llm with a utility LLM)",
        ))
    }

    fn name(&self) -> &'static str {
        "DisabledDecisionsService"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disabled_service_reports_itself_and_rejects_requests() {
        let service = DisabledDecisionsService;
        assert!(!service.is_configured());
        let error = service
            .evaluate(DecisionRequest::new("anything").ask("q", DecisionQuestion::noul("Yes?")))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("UTILITY_TYPESAFE_API_KEY"),
            "a disabled service must name the switch that enables it: {error}"
        );
    }

    #[test]
    fn score_tail_mass_is_read_from_the_distribution_not_the_mean() {
        // Probably fine, possibly severe: the mean says 0.6, the tail says 30%.
        let answer = DecisionAnswer::Score {
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
    fn label_answers_put_all_mass_on_the_label() {
        assert_eq!(
            DecisionAnswer::noul_label(true).probability_yes(),
            Some(1.0)
        );
        assert_eq!(
            DecisionAnswer::noul_label(false).probability_yes(),
            Some(0.0)
        );

        let DecisionAnswer::Choice {
            selected,
            probabilities,
            ..
        } = DecisionAnswer::choice_label("b", ["a", "b", "c"])
        else {
            panic!("expected a choice");
        };
        assert_eq!(selected, "b");
        assert_eq!(probabilities.len(), 3);
        assert_eq!(probabilities["b"], 1.0);
        assert_eq!(probabilities["a"], 0.0);

        let score = DecisionAnswer::score_label(2, 3);
        assert_eq!(score.probability_at_or_above(2), Some(1.0));
        assert_eq!(score.probability_at_or_above(0), Some(1.0));
        let DecisionAnswer::Score { score, .. } = score else {
            panic!("expected a score");
        };
        assert_eq!(score, 2.0);
    }

    #[test]
    fn noul_answers_have_no_separate_confidence() {
        let answer = DecisionAnswer::Noul { probability: 0.92 };
        assert_eq!(answer.probability_yes(), Some(0.92));
        assert_eq!(answer.confidence(), None);
        assert_eq!(answer.probability_at_or_above(1), None);
    }
}
