//! Portable evaluation vocabulary shared by hosted runs and application evaluators.
use crate::typed_id::ModelId;
use serde::{Deserialize, Serialize};
/// A scoring rule applied to eval case output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Scorer {
    /// Final assistant message contains substring.
    Contains {
        text: String,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Final assistant message does NOT contain substring.
    NotContains {
        text: String,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Final assistant message matches regex pattern.
    Regex {
        pattern: String,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Agent called named tool at least `min` times. With
    /// `arguments_contain`, only calls whose argument JSON contains that text
    /// count, so a case can check what the agent asked a tool to do.
    ToolCalled {
        tool: String,
        #[serde(default = "default_min_one")]
        min: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments_contain: Option<String>,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Agent did NOT call named tool.
    ToolNotCalled {
        tool: String,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Total tool calls within range.
    ToolCallCount {
        #[serde(skip_serializing_if = "Option::is_none")]
        min: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max: Option<u32>,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Completed within N turns.
    TurnsWithin {
        max: u32,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// The agent spent at most `max` tokens (input plus output).
    TokensWithin {
        #[cfg_attr(feature = "openapi", schema(example = 20000))]
        max: u64,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Session filesystem file contains substring.
    FileContains {
        path: String,
        text: String,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Final assistant message parses as JSON matching schema.
    JsonSchema {
        schema: serde_json::Value,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// Citation faithfulness: the answer's citations
    /// must cover the claim and be verified as supported. Scored from the
    /// `TextAnnotation`s on the final message — pair with the
    /// `citation_verification` capability so verdicts are present.
    CitationFaithful {
        /// Minimum number of citations the answer must carry.
        #[serde(default)]
        #[cfg_attr(feature = "openapi", schema(example = 1))]
        min_citations: u32,
        /// Minimum fraction of citations verified `entailed` to pass.
        #[serde(default = "default_pass_threshold")]
        #[cfg_attr(feature = "openapi", schema(example = 0.8))]
        pass_threshold: f64,
        /// Relative weight of this scorer in the case's weighted average.
        #[serde(default = "default_weight")]
        #[cfg_attr(feature = "openapi", schema(example = 1.0))]
        weight: f64,
    },
    /// Citation faithfulness judged by an LLM: each cited claim/source pair is
    /// graded by a model, so the eval works even without the
    /// `citation_verification` capability.
    CitationJudged {
        /// Rubric override; a citation-faithfulness rubric is used when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            feature = "openapi",
            schema(example = "Score the fraction of cited claims supported by their source.")
        )]
        rubric: Option<String>,
        /// Judge model; the org's default is used when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_id: Option<ModelId>,
        /// Minimum judged score `[0,1]` to pass.
        #[serde(default = "default_pass_threshold")]
        #[cfg_attr(feature = "openapi", schema(example = 0.8))]
        pass_threshold: f64,
        /// Relative weight of this scorer in the case's weighted average.
        #[serde(default = "default_weight")]
        #[cfg_attr(feature = "openapi", schema(example = 1.0))]
        weight: f64,
    },
    /// The org's chat model grades the answer against a rubric and returns a
    /// value from 0 to 1 with its reasoning.
    Judge {
        /// What a good answer does and what a bad one does.
        #[cfg_attr(
            feature = "openapi",
            schema(
                example = "Score 1 when the answer resolves the refund request with the policy's \
                             actual terms; 0 when it invents terms or deflects."
            )
        )]
        rubric: String,
        /// Judge model; the org's default is used when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_id: Option<ModelId>,
        /// Minimum judged value to pass.
        #[serde(default = "default_half")]
        #[cfg_attr(feature = "openapi", schema(example = 0.5))]
        pass_threshold: f64,
        #[serde(default = "default_weight")]
        weight: f64,
    },
    /// A decision model answers one yes/no question about the answer; the
    /// probability of yes is the value. Cheaper than `judge` and typed, so it
    /// suits any check with a yes/no shape.
    Decision {
        /// The yes/no question, asked about the user message, the answer and
        /// the tools used.
        #[cfg_attr(
            feature = "openapi",
            schema(example = "Does the answer stay within the published refund policy?")
        )]
        question: String,
        /// What a yes means, when the question alone is ambiguous.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        yes: Option<String>,
        /// What a no means.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        no: Option<String>,
        /// Minimum probability of yes to pass.
        #[serde(default = "default_half")]
        #[cfg_attr(feature = "openapi", schema(example = 0.5))]
        pass_threshold: f64,
        #[serde(default = "default_weight")]
        weight: f64,
    },
}

fn default_pass_threshold() -> f64 {
    0.8
}

fn default_half() -> f64 {
    0.5
}

impl Scorer {
    /// Stable kind tag for this scorer, matching the serde `type` discriminant.
    ///
    /// Scores are persisted as an ordered `Vec<Score>` that carries no scorer
    /// identity, so consumers that need a name (e.g. dataset export) join this
    /// positionally against the case's `scorers`. Keep this exhaustive so adding
    /// a variant forces a decision here.
    pub fn kind(&self) -> &'static str {
        match self {
            Scorer::Contains { .. } => "contains",
            Scorer::NotContains { .. } => "not_contains",
            Scorer::Regex { .. } => "regex",
            Scorer::ToolCalled { .. } => "tool_called",
            Scorer::ToolNotCalled { .. } => "tool_not_called",
            Scorer::ToolCallCount { .. } => "tool_call_count",
            Scorer::TurnsWithin { .. } => "turns_within",
            Scorer::TokensWithin { .. } => "tokens_within",
            Scorer::FileContains { .. } => "file_contains",
            Scorer::JsonSchema { .. } => "json_schema",
            Scorer::CitationFaithful { .. } => "citation_faithful",
            Scorer::CitationJudged { .. } => "citation_judged",
            Scorer::Judge { .. } => "judge",
            Scorer::Decision { .. } => "decision",
        }
    }

    /// Reject a scorer that could never grade: an empty rubric or question,
    /// a threshold outside 0..1, a pattern or schema that does not compile.
    /// Returns the reason, for a 400.
    pub fn validate(&self) -> std::result::Result<(), String> {
        let threshold = |name: &str, value: f64| {
            if (0.0..=1.0).contains(&value) {
                Ok(())
            } else {
                Err(format!("{name} pass_threshold must be between 0.0 and 1.0"))
            }
        };
        if !self.weight().is_finite() || self.weight() < 0.0 {
            return Err(format!("{} weight must be zero or more", self.kind()));
        }
        match self {
            Scorer::Judge {
                rubric,
                pass_threshold,
                ..
            } => {
                if rubric.trim().is_empty() {
                    return Err("judge rubric must not be empty".to_string());
                }
                threshold("judge", *pass_threshold)
            }
            Scorer::Decision {
                question,
                pass_threshold,
                ..
            } => {
                if question.trim().is_empty() {
                    return Err("decision question must not be empty".to_string());
                }
                threshold("decision", *pass_threshold)
            }
            Scorer::CitationFaithful { pass_threshold, .. } => {
                threshold("citation_faithful", *pass_threshold)
            }
            Scorer::CitationJudged { pass_threshold, .. } => {
                threshold("citation_judged", *pass_threshold)
            }
            Scorer::Regex { pattern, .. } => regex::Regex::new(pattern)
                .map(|_| ())
                .map_err(|error| format!("regex pattern does not compile: {error}")),
            Scorer::ToolCalled {
                arguments_contain: Some(text),
                ..
            } if text.is_empty() => {
                Err("tool_called arguments_contain must not be empty".to_string())
            }
            Scorer::JsonSchema { schema, .. } => jsonschema::validator_for(schema)
                .map(|_| ())
                .map_err(|error| format!("json_schema schema is invalid: {error}")),
            _ => Ok(()),
        }
    }

    /// Relative weight of this scorer in the case's weighted score.
    pub fn weight(&self) -> f64 {
        match self {
            Scorer::Contains { weight, .. }
            | Scorer::NotContains { weight, .. }
            | Scorer::Regex { weight, .. }
            | Scorer::ToolCalled { weight, .. }
            | Scorer::ToolNotCalled { weight, .. }
            | Scorer::ToolCallCount { weight, .. }
            | Scorer::TurnsWithin { weight, .. }
            | Scorer::TokensWithin { weight, .. }
            | Scorer::FileContains { weight, .. }
            | Scorer::JsonSchema { weight, .. }
            | Scorer::CitationFaithful { weight, .. }
            | Scorer::CitationJudged { weight, .. }
            | Scorer::Judge { weight, .. }
            | Scorer::Decision { weight, .. } => *weight,
        }
    }
}

fn default_weight() -> f64 {
    1.0
}
fn default_min_one() -> u32 {
    1
}
/// Result from a single scorer evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Score {
    /// Whether this scorer passed.
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub pass: bool,
    /// Score value 0.0–1.0.
    #[cfg_attr(feature = "openapi", schema(example = 1.0))]
    pub value: f64,
    /// Human-readable explanation.
    #[cfg_attr(feature = "openapi", schema(example = "Output contains expected text"))]
    pub reason: String,
}
