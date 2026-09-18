//! Typed answers, and the accessors that turn them into decisions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::client::{Error, Result};

/// One answer, matching the type of the question that produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    /// Answer to a [`crate::Question::noul`].
    Noul(NoulAnswer),
    /// Answer to a [`crate::Question::choice`].
    Choice(ChoiceAnswer),
    /// Answer to a [`crate::Question::score`].
    Score(ScoreAnswer),
}

/// A yes/no answer: the probability that the answer is yes.
///
/// A value near 0.5 means yes and no are close to equally likely — not "medium
/// intensity", and not low confidence in a middling verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoulAnswer {
    /// Probability of yes, from 0 (no) to 1 (yes).
    pub noul: f64,
}

/// A selected option plus the full distribution over the options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceAnswer {
    /// The highest-probability option.
    pub choice: String,
    /// Every option mapped to its probability.
    #[serde(default)]
    pub probabilities: BTreeMap<String, f64>,
    /// How concentrated the distribution is, from 0 to 1.
    #[serde(default)]
    pub confidence: f64,
}

/// A probability-weighted position along the levels, plus the distribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreAnswer {
    /// The weighted position across levels; lands between levels.
    pub score: f64,
    /// Level index (as a string) to the description it was given.
    #[serde(default)]
    pub legend: BTreeMap<String, String>,
    /// Level index (as a string) to its probability.
    #[serde(default)]
    pub probabilities: BTreeMap<String, f64>,
    /// How concentrated the distribution is, from 0 to 1.
    #[serde(default)]
    pub confidence: f64,
}

impl Answer {
    /// The primitive name, as it appears on the wire.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Noul(_) => "noul",
            Self::Choice(_) => "choice",
            Self::Score(_) => "score",
        }
    }

    /// Distribution concentration, for the two primitives that report it.
    ///
    /// A noul has no separate confidence: its probability already carries it.
    pub fn confidence(&self) -> Option<f64> {
        match self {
            Self::Noul(_) => None,
            Self::Choice(c) => Some(c.confidence),
            Self::Score(s) => Some(s.confidence),
        }
    }
}

impl NoulAnswer {
    /// Probability that the answer is yes, 0..=1.
    ///
    /// The readable name for the [`NoulAnswer::noul`] field.
    pub fn probability(&self) -> f64 {
        self.noul
    }
}

impl ChoiceAnswer {
    /// Probability assigned to one option, 0 when the option is unknown.
    pub fn probability_of(&self, option: &str) -> f64 {
        self.probabilities.get(option).copied().unwrap_or(0.0)
    }
}

impl ScoreAnswer {
    /// How many levels the question defined.
    pub fn level_count(&self) -> usize {
        self.probabilities.len().max(self.legend.len())
    }

    /// The score mapped onto 0.0..=1.0, so thresholds survive a change in the
    /// number of levels.
    ///
    /// Returns 0.0 for a degenerate single-level question.
    pub fn normalized(&self) -> f64 {
        match self.level_count() {
            0 | 1 => 0.0,
            levels => (self.score / (levels - 1) as f64).clamp(0.0, 1.0),
        }
    }

    /// The level the score is nearest to.
    pub fn nearest_level(&self) -> usize {
        self.score.round().max(0.0) as usize
    }

    /// The description of the level the score is nearest to.
    pub fn nearest_label(&self) -> Option<&str> {
        self.legend
            .get(&self.nearest_level().to_string())
            .map(String::as_str)
    }

    /// Probability of one level.
    pub fn probability_of(&self, level: usize) -> f64 {
        self.probabilities
            .get(&level.to_string())
            .copied()
            .unwrap_or(0.0)
    }

    /// Total probability mass at `level` or above.
    ///
    /// This is the honest reading for "how likely is this at least as bad as
    /// level N", where the weighted score would understate a bimodal answer.
    pub fn probability_at_or_above(&self, level: usize) -> f64 {
        self.probabilities
            .iter()
            .filter(|(key, _)| key.parse::<usize>().is_ok_and(|index| index >= level))
            .map(|(_, probability)| probability)
            .sum()
    }
}

/// Token usage for one request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens consumed by the state and questions.
    #[serde(default)]
    pub input_tokens: u64,
    /// Tokens produced by the model.
    #[serde(default)]
    pub output_tokens: u64,
}

/// The result of one evaluation: every answer, keyed by the ids you asked under.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Judgment {
    /// The model that performed the evaluation, as resolved by the API.
    #[serde(default)]
    pub model: String,
    /// One answer per question.
    #[serde(default)]
    pub answers: BTreeMap<String, Answer>,
    /// Token usage for the request.
    #[serde(default)]
    pub usage: Usage,
}

impl Judgment {
    /// The raw answer under `id`, if the response carries one.
    pub fn get(&self, id: &str) -> Option<&Answer> {
        self.answers.get(id)
    }

    /// Probability of yes for a noul question.
    pub fn noul(&self, id: &str) -> Result<f64> {
        match self.expect(id)? {
            Answer::Noul(answer) => Ok(answer.noul),
            other => Err(self.mismatch(id, "noul", other)),
        }
    }

    /// Probability of yes for a yes/no question.
    ///
    /// Alias of [`Judgment::noul`], under the name other TypeSafe clients use.
    pub fn probability(&self, id: &str) -> Result<f64> {
        self.noul(id)
    }

    /// The answer to a choice question.
    pub fn choice(&self, id: &str) -> Result<&ChoiceAnswer> {
        match self.expect(id)? {
            Answer::Choice(answer) => Ok(answer),
            other => Err(self.mismatch(id, "choice", other)),
        }
    }

    /// The answer to a score question.
    pub fn score(&self, id: &str) -> Result<&ScoreAnswer> {
        match self.expect(id)? {
            Answer::Score(answer) => Ok(answer),
            other => Err(self.mismatch(id, "score", other)),
        }
    }

    fn expect(&self, id: &str) -> Result<&Answer> {
        self.answers
            .get(id)
            .ok_or_else(|| Error::UnknownAnswer(id.to_string()))
    }

    fn mismatch(&self, id: &str, expected: &'static str, actual: &Answer) -> Error {
        Error::AnswerType {
            id: id.to_string(),
            expected,
            actual: actual.kind(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The accessors' happy paths are covered by `tests/client.rs` against a
    // mock endpoint. These cover the defensive branches instead: the shapes a
    // vendor response can take that no well-formed answer produces, where a
    // wrong fallback reads as a real verdict rather than as a missing one.

    fn score(score: f64, probabilities: &[(&str, f64)], legend: &[(&str, &str)]) -> ScoreAnswer {
        ScoreAnswer {
            score,
            legend: legend
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            probabilities: probabilities
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
            confidence: 0.5,
        }
    }

    /// A single-level (or level-less) question has no range to normalize over.
    /// Dividing by `levels - 1` there would be a divide-by-zero reading as NaN
    /// or infinity, and a threshold compared against either silently passes.
    #[test]
    fn a_degenerate_level_count_normalizes_to_zero_not_nan() {
        assert_eq!(score(0.0, &[], &[]).normalized(), 0.0);
        assert_eq!(
            score(0.0, &[("0", 1.0)], &[("0", "Only")]).normalized(),
            0.0
        );
    }

    /// A score outside the declared range would otherwise normalize past 1.0
    /// and clear every threshold, or below 0.0 and clear none.
    #[test]
    fn normalization_clamps_a_score_outside_the_declared_levels() {
        let levels = [("0", 0.0), ("1", 0.0), ("2", 1.0)];
        assert_eq!(score(9.0, &levels, &[]).normalized(), 1.0);
        assert_eq!(score(-3.0, &levels, &[]).normalized(), 0.0);
    }

    /// `nearest_level` indexes levels, so a negative score floors at 0 rather
    /// than wrapping to a huge `usize` on the cast.
    #[test]
    fn a_negative_score_lands_on_the_lowest_level() {
        let answer = score(-0.4, &[("0", 1.0)], &[("0", "Calm")]);
        assert_eq!(answer.nearest_level(), 0);
        assert_eq!(answer.nearest_label(), Some("Calm"));
    }

    /// Either map alone establishes the range, so a response that carries only
    /// a legend still normalizes over the right number of levels.
    #[test]
    fn level_count_takes_the_wider_of_legend_and_distribution() {
        let answer = score(1.0, &[("0", 1.0)], &[("0", "A"), ("1", "B"), ("2", "C")]);
        assert_eq!(answer.level_count(), 3);
        assert_eq!(answer.normalized(), 0.5);
    }

    /// An unknown level is absent, not impossible-but-present: 0.0 is the
    /// honest reading, and it must not join a tail-mass sum.
    #[test]
    fn unknown_and_unparseable_levels_contribute_nothing() {
        let answer = score(1.0, &[("0", 0.2), ("1", 0.3), ("oops", 0.9)], &[]);
        assert_eq!(answer.probability_of(7), 0.0);
        assert!((answer.probability_at_or_above(0) - 0.5).abs() < 1e-9);
        assert!((answer.probability_at_or_above(1) - 0.3).abs() < 1e-9);
        assert_eq!(answer.probability_at_or_above(2), 0.0);
    }

    #[test]
    fn a_missing_label_is_none_rather_than_an_empty_string() {
        assert_eq!(score(2.0, &[], &[("0", "Calm")]).nearest_label(), None);
    }

    #[test]
    fn an_unknown_choice_option_has_no_probability() {
        let answer = ChoiceAnswer {
            choice: "billing".to_string(),
            probabilities: [("billing".to_string(), 1.0)].into_iter().collect(),
            confidence: 0.9,
        };
        assert_eq!(answer.probability_of("billing"), 1.0);
        assert_eq!(answer.probability_of("technical"), 0.0);
    }

    /// A noul's probability already carries its confidence, so reporting a
    /// separate number would invite thresholding on a value that is not there.
    #[test]
    fn a_noul_reports_no_separate_confidence() {
        assert_eq!(Answer::Noul(NoulAnswer { noul: 0.5 }).confidence(), None);
        assert_eq!(Answer::Noul(NoulAnswer { noul: 0.5 }).kind(), "noul");
        assert_eq!(NoulAnswer { noul: 0.25 }.probability(), 0.25);
    }

    /// Reading an answer as the wrong primitive names both types, because the
    /// caller's next move is to fix the question or the accessor.
    #[test]
    fn a_type_mismatch_names_what_was_asked_for_and_what_arrived() {
        let judgment = Judgment {
            model: "jev-1.13.0".to_string(),
            answers: [("q".to_string(), Answer::Noul(NoulAnswer { noul: 0.4 }))]
                .into_iter()
                .collect(),
            usage: Usage::default(),
        };
        assert_eq!(judgment.probability("q").unwrap(), 0.4);

        let error = judgment.score("q").unwrap_err();
        assert!(matches!(
            error,
            Error::AnswerType {
                expected: "score",
                actual: "noul",
                ..
            }
        ));
        assert!(matches!(judgment.noul("absent"), Err(Error::UnknownAnswer(id)) if id == "absent"));
    }
}
