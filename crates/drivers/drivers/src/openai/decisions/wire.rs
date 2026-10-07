//! Request and response bodies for `POST /v1/decisions`.
//!
//! Matches the published reference
//! (<https://developers.openai.com/api/docs/guides/decisions>, public beta)
//! and was checked against live responses on 2026-10-06. Limits the endpoint
//! enforces that the guide does not list, probed the same day: names must be
//! unique within a request, a choice needs at least two choices, and a score
//! takes at most ten levels. A question the model declines comes back as a
//! `refusal` answer with HTTP 200.

use serde::{Deserialize, Serialize};

/// One request: shared input and every question about it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecisionBody {
    /// Model id.
    pub model: String,
    /// The evidence the questions are about, as text.
    pub input: String,
    /// The questions, answered together.
    pub questions: Vec<QuestionBody>,
}

/// One question, tagged by its `type`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuestionBody {
    /// Whether a condition holds; answered with a probability.
    Predicate {
        /// Echoed on the answer.
        name: String,
        /// The condition.
        instructions: String,
    },
    /// One value from a fixed set.
    Choice {
        /// Echoed on the answer.
        name: String,
        /// What to decide.
        instructions: String,
        /// At least two.
        choices: Vec<ChoiceBody>,
    },
    /// A position along ordered levels, lowest first.
    Score {
        /// Echoed on the answer.
        name: String,
        /// What to rate.
        instructions: String,
        /// At most ten; indices start at 0.
        levels: Vec<LevelBody>,
    },
}

impl QuestionBody {
    /// The name the answer comes back under.
    pub fn name(&self) -> &str {
        match self {
            Self::Predicate { name, .. } | Self::Choice { name, .. } | Self::Score { name, .. } => {
                name
            }
        }
    }
}

/// One allowed choice.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChoiceBody {
    /// Returned as the answer's `choice` when picked.
    pub value: String,
    /// When this choice applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One score level.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LevelBody {
    /// Short level name.
    pub label: String,
    /// The level's criteria.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The response.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct DecisionResponse {
    /// The model that answered.
    #[serde(default)]
    pub model: Option<String>,
    /// One answer per question, carrying its name.
    #[serde(default)]
    pub answers: Vec<AnswerBody>,
    /// Token usage for the whole request.
    #[serde(default)]
    pub usage: Usage,
}

/// One answer, tagged by its question's `type`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnswerBody {
    /// Probability the condition is true.
    Predicate {
        /// The question's name.
        name: String,
        /// 0..=1.
        probability: f64,
    },
    /// The picked value and the distribution over choices.
    Choice {
        /// The question's name.
        name: String,
        /// One of the supplied values.
        choice: String,
        /// Probability per choice.
        #[serde(default)]
        probabilities: Vec<ChoiceProbability>,
        /// The API's confidence in its pick.
        #[serde(default)]
        confidence: Option<f64>,
    },
    /// Probability-weighted level index and the distribution over levels.
    Score {
        /// The question's name.
        name: String,
        /// Weighted average of level indices.
        score: f64,
        /// Probability per level.
        #[serde(default)]
        probabilities: Vec<LevelProbability>,
        /// The API's confidence.
        #[serde(default)]
        confidence: Option<f64>,
    },
    /// The model declined to answer.
    Refusal {
        /// The question's name.
        name: String,
    },
}

impl AnswerBody {
    /// The question name this answer belongs to.
    pub fn name(&self) -> &str {
        match self {
            Self::Predicate { name, .. }
            | Self::Choice { name, .. }
            | Self::Score { name, .. }
            | Self::Refusal { name } => name,
        }
    }
}

/// One choice's probability.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChoiceProbability {
    /// The choice value.
    pub value: String,
    /// 0..=1.
    pub probability: f64,
}

/// One level's probability.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LevelProbability {
    /// Level index, from 0.
    pub value: usize,
    /// 0..=1.
    pub probability: f64,
}

/// Token usage. The API bills input tokens only; output is reported as 0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct Usage {
    /// Input tokens.
    #[serde(default)]
    pub input_tokens: u64,
    /// Output tokens.
    #[serde(default)]
    pub output_tokens: u64,
}

/// OpenAI's standard error envelope.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ErrorEnvelope {
    /// The error.
    #[serde(default)]
    pub error: ErrorBody,
}

/// The error inside the envelope.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ErrorBody {
    /// Human-readable message.
    #[serde(default)]
    pub message: String,
    /// Error type, such as `invalid_request_error`.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

/// Parse a success body.
pub fn parse_response(body: &serde_json::Value) -> Option<DecisionResponse> {
    serde_json::from_value(body.clone()).ok()
}
