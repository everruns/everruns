//! Request and response bodies for `POST /v1/decisions`.
//!
//! PROVISIONAL. OpenAI has not published a reference for the Decisions API
//! (limited preview since DevDay, 2026-09-29), and the endpoint refuses our
//! account with "Decision API is not enabled for this user". What is known:
//! the path (`/v1/decisions`, confirmed by that refusal), that it takes a
//! context plus a closed list of answers, that it answers with the chosen
//! answer and a confidence, and that it runs on GPT-6 Luna. Everything else
//! here is inferred. It is isolated in this file so that verifying it against
//! the reference changes one module and its fixtures, and parsing is lenient
//! (field aliases, optional distribution) so a near miss degrades to an
//! uncalibrated label rather than a parse failure.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One decision: a single question over a closed set of answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecisionBody {
    /// Model id.
    pub model: String,
    /// The context being decided about, as text.
    pub input: String,
    /// The question.
    pub instructions: String,
    /// The answers the model may pick from.
    pub options: Vec<OptionBody>,
}

/// One allowed answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OptionBody {
    /// The value returned when this answer is picked.
    pub label: String,
    /// What the answer means, when the caller said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The chosen answer.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct DecisionResponse {
    /// The model that answered.
    #[serde(default)]
    pub model: Option<String>,
    /// The picked label.
    #[serde(alias = "decision", alias = "answer", alias = "choice")]
    pub label: String,
    /// Confidence in the picked label, 0..=1.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Probability per label, when the API returns a full distribution.
    #[serde(default, alias = "scores")]
    pub probabilities: Option<BTreeMap<String, f64>>,
    /// Token usage.
    #[serde(default)]
    pub usage: Usage,
}

/// Token usage for one decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct Usage {
    /// Input tokens.
    #[serde(default, alias = "prompt_tokens")]
    pub input_tokens: u64,
    /// Output tokens.
    #[serde(default, alias = "completion_tokens")]
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

/// The response, whether it is the object itself or wraps it in `output`.
pub fn parse_response(body: &serde_json::Value) -> Option<DecisionResponse> {
    let mut parsed: DecisionResponse = serde_json::from_value(body.clone())
        .ok()
        .or_else(|| serde_json::from_value(body.get("output")?.clone()).ok())?;
    if parsed.model.is_none() {
        parsed.model = body.get("model").and_then(|m| m.as_str()).map(String::from);
    }
    if parsed.usage == Usage::default()
        && let Some(usage) = body.get("usage")
    {
        parsed.usage = serde_json::from_value(usage.clone()).unwrap_or_default();
    }
    Some(parsed)
}
