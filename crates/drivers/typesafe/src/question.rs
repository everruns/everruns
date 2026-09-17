//! The three System One question types and the request that carries them.
//!
//! A question is a judgment you want back as a value, not as prose. Pick by
//! what the answer *means*:
//!
//! - [`Question::noul`] — whether a condition holds, as the probability of yes.
//! - [`Question::choice`] — exactly one option out of a set you define.
//! - [`Question::score`] — a position along ordered levels you describe.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

/// TypeSafe's flagship System One model.
pub const DEFAULT_MODEL: &str = "jev-latest";

/// One typed question.
///
/// `instructions` and the criteria accept plain strings or arbitrary JSON, so
/// definitions, contrasts, and examples can be given structure when a sentence
/// is not enough.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    /// Yes/no, answered as the probability of yes.
    Noul {
        /// The yes/no question to evaluate.
        instructions: Value,
        /// Optional descriptions of what yes and no mean.
        #[serde(skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
    /// One option from a defined set.
    Choice {
        /// What the model should decide.
        instructions: Value,
        /// Option to rubric description; `null` when an option needs no detail.
        criteria: BTreeMap<String, Option<Value>>,
    },
    /// A position along ordered levels.
    Score {
        /// What the model should rate.
        instructions: Value,
        /// Ordered level descriptions, lowest first. At least two.
        criteria: Vec<Value>,
    },
}

/// What a yes and a no mean for a [`Question::Noul`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct NoulCriteria {
    /// What a yes (value near 1) means.
    #[serde(rename = "true", skip_serializing_if = "Option::is_none")]
    pub yes: Option<String>,
    /// What a no (value near 0) means.
    #[serde(rename = "false", skip_serializing_if = "Option::is_none")]
    pub no: Option<String>,
}

impl Question {
    /// A yes/no question.
    ///
    /// ```
    /// use typesafe_systemone::Question;
    /// let q = Question::noul("Does this message convey urgency?");
    /// ```
    pub fn noul(instructions: impl Into<Value>) -> Self {
        Self::Noul {
            instructions: instructions.into(),
            criteria: None,
        }
    }

    /// A yes/no question, named as other TypeSafe clients name it.
    ///
    /// Alias of [`Question::noul`]; `noul` is the name on the wire and in
    /// TypeSafe's own docs, `boolean` is the one the AI SDK uses.
    pub fn boolean(instructions: impl Into<Value>) -> Self {
        Self::noul(instructions)
    }

    /// Describe what yes and no mean for a [`Question::noul`]. No-op on the
    /// other question types.
    pub fn criteria(mut self, yes: impl Into<String>, no: impl Into<String>) -> Self {
        if let Self::Noul { criteria, .. } = &mut self {
            *criteria = Some(NoulCriteria {
                yes: Some(yes.into()),
                no: Some(no.into()),
            });
        }
        self
    }

    /// A single-selection question over the given options.
    ///
    /// ```
    /// use typesafe_systemone::Question;
    /// let q = Question::choice(
    ///     "Which team should handle this?",
    ///     [("billing", "Payments, invoicing, refunds"), ("technical", "Bugs and outages")],
    /// );
    /// ```
    pub fn choice<K, V, I>(instructions: impl Into<Value>, options: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<Value>,
    {
        Self::Choice {
            instructions: instructions.into(),
            criteria: options
                .into_iter()
                .map(|(option, rubric)| (option.into(), Some(rubric.into())))
                .collect(),
        }
    }

    /// A single-selection question over bare options, with no per-option rubric.
    pub fn choice_of<K, I>(instructions: impl Into<Value>, options: I) -> Self
    where
        I: IntoIterator<Item = K>,
        K: Into<String>,
    {
        Self::Choice {
            instructions: instructions.into(),
            criteria: options.into_iter().map(|o| (o.into(), None)).collect(),
        }
    }

    /// A graded question over ordered levels, lowest first.
    ///
    /// ```
    /// use typesafe_systemone::Question;
    /// let q = Question::score(
    ///     "How funny is this joke?",
    ///     ["Not funny at all", "Mildly amusing", "Genuinely funny", "Hilarious"],
    /// );
    /// ```
    pub fn score<L, I>(instructions: impl Into<Value>, levels: I) -> Self
    where
        I: IntoIterator<Item = L>,
        L: Into<Value>,
    {
        Self::Score {
            instructions: instructions.into(),
            criteria: levels.into_iter().map(Into::into).collect(),
        }
    }

    /// The primitive name, as it appears on the wire.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Noul { .. } => "noul",
            Self::Choice { .. } => "choice",
            Self::Score { .. } => "score",
        }
    }

    /// Reject questions the API would reject, before spending a round trip.
    pub(crate) fn validate(&self, id: &str) -> crate::Result<()> {
        let invalid = |msg: String| Err(crate::Error::InvalidRequest(msg));
        match self {
            Self::Noul { instructions, .. } | Self::Choice { instructions, .. } => {
                if is_blank(instructions) {
                    return invalid(format!("question '{id}' has empty instructions"));
                }
                if let Self::Choice { criteria, .. } = self
                    && criteria.len() < 2
                {
                    return invalid(format!(
                        "choice question '{id}' needs at least two options, got {}",
                        criteria.len()
                    ));
                }
                Ok(())
            }
            Self::Score {
                instructions,
                criteria,
            } => {
                if is_blank(instructions) {
                    return invalid(format!("question '{id}' has empty instructions"));
                }
                if criteria.len() < 2 {
                    return invalid(format!(
                        "score question '{id}' needs at least two levels, got {}",
                        criteria.len()
                    ));
                }
                Ok(())
            }
        }
    }
}

fn is_blank(instructions: &Value) -> bool {
    match instructions {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
        _ => false,
    }
}

/// One evaluation: the state to judge plus the questions to ask about it.
///
/// Questions asked together run in parallel inside a single request and cannot
/// see one another's answers. Batching is the cheap path — ask everything the
/// code might need, including questions only one branch will read.
#[derive(Debug, Clone, Serialize)]
pub struct Evaluation {
    /// The content being judged: text, or structured data.
    pub state: Value,
    /// The model handling the request.
    pub model: String,
    /// Questions keyed by ids you choose. Ids are for your code — they are not
    /// sent to the model — so the question itself must carry its full meaning.
    pub questions: BTreeMap<String, Question>,
}

impl Evaluation {
    /// Start an evaluation over `state`, which may be a string or any JSON.
    pub fn new(state: impl Into<Value>) -> Self {
        Self {
            state: state.into(),
            model: DEFAULT_MODEL.to_string(),
            questions: BTreeMap::new(),
        }
    }

    /// Add a question under `id`. Re-using an id replaces the question.
    pub fn ask(mut self, id: impl Into<String>, question: Question) -> Self {
        self.questions.insert(id.into(), question);
        self
    }

    /// Pin a model other than [`DEFAULT_MODEL`].
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
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

    pub(crate) fn validate(&self) -> crate::Result<()> {
        if self.questions.is_empty() {
            return Err(crate::Error::InvalidRequest(
                "an evaluation must carry at least one question".to_string(),
            ));
        }
        if self.model.trim().is_empty() {
            return Err(crate::Error::InvalidRequest(
                "model must not be empty".to_string(),
            ));
        }
        for (id, question) in &self.questions {
            question.validate(id)?;
        }
        Ok(())
    }
}
