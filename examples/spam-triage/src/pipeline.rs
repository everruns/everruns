//! Two stages: a classifier screens every email, and only the uncertain ones
//! reach a model.
//!
//! The split is the point. A classifier answers a yes/no question as a
//! probability, so "how sure are you" is a number this code can branch on
//! rather than a hedge buried in prose. Everything it is sure about is settled
//! for a fraction of a cent; the remainder — where the probability sits near
//! the middle and a wrong answer is plausible — is worth a slower, larger
//! model.

use std::fmt;

use everruns::{Classifier, ClassifierError, CompletionError, Model, ReasoningEffort};
use futures::stream::{self, StreamExt, TryStreamExt};

use crate::dataset::{Email, Label};

/// TypeSafe's classifier, by alias. The answer reports the `jev-*` release that
/// resolved it — the id to pin once a threshold is calibrated against it.
pub const SCREENING_MODEL: &str = "jev-latest";

/// The larger model the uncertain cases are routed to, as OpenRouter names it.
pub const ADJUDICATION_MODEL: &str = "meta/muse-spark-1.3-contributor";

/// Below this confidence the screen does not decide.
///
/// Calibrated against this corpus and the `jev-*` release behind `jev-latest`
/// at the time of writing: every email the screen got wrong sat at 0.73
/// confidence or below, so 0.80 escalates all of them while leaving four fifths
/// of the corpus settled. A floor is evidence about one classifier version and
/// one mail mix; `--threshold` exists because yours will differ.
pub const DEFAULT_CONFIDENCE_FLOOR: f64 = 0.80;

/// In-flight requests per stage. Both services answer one email per request,
/// so the wall clock is set by how many are allowed to overlap.
pub const CONCURRENCY: usize = 8;

const QUESTION_ID: &str = "spam";
const SPAM_MEANS: &str = "Unsolicited bulk mail: advertising, chain letters, adult or medical offers, \
     get-rich schemes, or a forged sender trying to extract money or credentials.";
const LEGITIMATE_MEANS: &str = "Mail this recipient would expect: personal or work correspondence, a mailing \
     list or newsletter they subscribed to, a receipt, or an automated notice from \
     a service they use. Promotional tone alone does not make it spam.";

/// Why one email could not be triaged.
#[derive(Debug)]
pub enum TriageError {
    /// The classifier call failed.
    Screening(ClassifierError),
    /// The adjudicating model call failed.
    Adjudication(CompletionError),
}

impl fmt::Display for TriageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TriageError::Screening(error) => write!(f, "screening failed: {error}"),
            TriageError::Adjudication(error) => write!(f, "adjudication failed: {error}"),
        }
    }
}

impl std::error::Error for TriageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TriageError::Screening(error) => Some(error),
            TriageError::Adjudication(error) => Some(error),
        }
    }
}

impl From<ClassifierError> for TriageError {
    fn from(error: ClassifierError) -> Self {
        TriageError::Screening(error)
    }
}

impl From<CompletionError> for TriageError {
    fn from(error: CompletionError) -> Self {
        TriageError::Adjudication(error)
    }
}

/// What the classifier said about one email.
#[derive(Debug, Clone, Copy)]
pub struct Screening {
    /// Probability that the email is spam. Near 0.5 means spam and legitimate
    /// are near-equally likely — not "medium spam".
    pub spam_probability: f64,
    /// Tokens the classifier consumed for this email.
    pub input_tokens: u64,
}

impl Screening {
    /// The side of 0.5 the probability landed on.
    pub fn verdict(&self) -> Label {
        if self.spam_probability >= 0.5 {
            Label::Spam
        } else {
            Label::Legitimate
        }
    }

    /// How sure the screen is of its own verdict: the probability mass behind
    /// the side it picked.
    pub fn confidence(&self) -> f64 {
        self.spam_probability.max(1.0 - self.spam_probability)
    }

    /// Whether this email is settled without a second opinion.
    pub fn is_settled(&self, floor: f64) -> bool {
        self.confidence() >= floor
    }
}

/// What the larger model said about one escalated email.
#[derive(Debug, Clone)]
pub struct Adjudication {
    /// `None` when the answer did not name either verdict; the screen's stands.
    pub verdict: Option<Label>,
    /// Tokens the provider reported for this call.
    pub total_tokens: Option<u32>,
    /// Cost the provider reported inline, when it reports one. OpenRouter does.
    pub cost_usd: Option<f64>,
}

/// One email, through as much of the pipeline as it needed.
#[derive(Debug, Clone)]
pub struct Triage {
    pub email: Email,
    pub screening: Screening,
    /// `Some` only for emails the screen left undecided.
    pub adjudication: Option<Adjudication>,
}

impl Triage {
    /// The pipeline's answer: the escalation when there was one and it was
    /// readable, otherwise the screen's.
    pub fn verdict(&self) -> Label {
        self.adjudication
            .as_ref()
            .and_then(|adjudication| adjudication.verdict)
            .unwrap_or_else(|| self.screening.verdict())
    }

    /// Whether this email reached the second stage.
    pub fn escalated(&self) -> bool {
        self.adjudication.is_some()
    }

    /// Graded against the corpus label, which neither stage was shown.
    pub fn correct(&self) -> bool {
        self.verdict() == self.email.label
    }
}

/// Screen every email, `CONCURRENCY` requests in flight, results in input order.
pub async fn screen_all(
    classifier: &Classifier,
    emails: &[Email],
) -> Result<Vec<Screening>, TriageError> {
    stream::iter(emails.iter().map(|email| screen(classifier, email)))
        .buffered(CONCURRENCY)
        .try_collect()
        .await
}

/// Ask the one question the branch depends on, and read it as a number.
async fn screen(classifier: &Classifier, email: &Email) -> Result<Screening, TriageError> {
    let answers = classifier
        .about(email.as_state())
        .noul_between(
            QUESTION_ID,
            "Is this email spam?",
            SPAM_MEANS,
            LEGITIMATE_MEANS,
        )
        .send()
        .await?;
    Ok(Screening {
        spam_probability: answers.probability(QUESTION_ID)?,
        input_tokens: answers.outcome().usage.input_tokens,
    })
}

/// Route the undecided emails to the larger model, results in input order.
pub async fn adjudicate_all(
    model: &Model,
    emails: &[&Email],
) -> Result<Vec<Adjudication>, TriageError> {
    stream::iter(emails.iter().map(|email| adjudicate(model, email)))
        .buffered(CONCURRENCY)
        .try_collect()
        .await
}

/// One prompt, one word. No agent: there is no history to keep and no tool to
/// call, so a direct completion is the whole of what this stage needs.
async fn adjudicate(model: &Model, email: &Email) -> Result<Adjudication, TriageError> {
    let response = model
        .completion()
        .system(include_str!("resources/adjudicator.md"))
        .user(email.as_state())
        .temperature(0.0)
        // Muse reasons before it answers and will not let that be switched off,
        // so the budget has to cover the reasoning as well as the one word that
        // follows it. Too tight a cap returns an empty answer, which lands in
        // the report as an unreadable escalation rather than a wrong verdict.
        .reasoning_effort(ReasoningEffort::Low)
        .max_tokens(2048)
        .send()
        .await?;
    Ok(Adjudication {
        verdict: parse_verdict(&response.text),
        total_tokens: response.metadata.total_tokens,
        cost_usd: response.metadata.provider_cost_usd,
    })
}

/// Read a one-word verdict, tolerating the decoration a model sometimes adds.
///
/// The first word decides. Only when it names neither verdict does the rest of
/// the answer count, and then only if it names exactly one — "not spam" must
/// never be read as a spam verdict.
fn parse_verdict(answer: &str) -> Option<Label> {
    let first_word: String = answer
        .trim_start_matches(|character: char| !character.is_ascii_alphabetic())
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();
    match first_word.to_ascii_uppercase().as_str() {
        "SPAM" => return Some(Label::Spam),
        "LEGITIMATE" => return Some(Label::Legitimate),
        _ => {}
    }

    let lowered = answer.to_ascii_lowercase();
    match (lowered.contains("spam"), lowered.contains("legitimate")) {
        (true, false) => Some(Label::Spam),
        (false, true) => Some(Label::Legitimate),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screening(spam_probability: f64) -> Screening {
        Screening {
            spam_probability,
            input_tokens: 0,
        }
    }

    #[test]
    fn confidence_is_the_mass_behind_the_verdict_not_the_probability() {
        assert_eq!(screening(0.02).verdict(), Label::Legitimate);
        assert!((screening(0.02).confidence() - 0.98).abs() < 1e-9);
        assert_eq!(screening(0.98).verdict(), Label::Spam);
        assert!((screening(0.98).confidence() - 0.98).abs() < 1e-9);
    }

    #[test]
    fn only_the_uncertain_middle_is_escalated() {
        let floor = DEFAULT_CONFIDENCE_FLOOR;
        assert!(screening(0.99).is_settled(floor));
        assert!(screening(0.01).is_settled(floor));
        assert!(!screening(0.6).is_settled(floor));
        assert!(!screening(0.4).is_settled(floor));
        // Exactly at the floor is settled; the boundary is not escalated twice.
        assert!(screening(0.9).is_settled(0.9));
        assert!(!screening(0.9).is_settled(0.91));
    }

    #[test]
    fn a_one_word_answer_is_the_verdict() {
        assert_eq!(parse_verdict("SPAM"), Some(Label::Spam));
        assert_eq!(parse_verdict("legitimate\n"), Some(Label::Legitimate));
        assert_eq!(parse_verdict("**SPAM**"), Some(Label::Spam));
        assert_eq!(parse_verdict("  LEGITIMATE."), Some(Label::Legitimate));
    }

    #[test]
    fn a_wordy_answer_falls_back_to_the_one_verdict_it_names() {
        assert_eq!(
            parse_verdict("This is clearly SPAM."),
            Some(Label::Spam),
            "a single named verdict is readable"
        );
        assert_eq!(
            parse_verdict("Verdict: not spam, it is legitimate"),
            None,
            "naming both must not be resolved by guessing"
        );
        assert_eq!(parse_verdict("I cannot tell."), None);
        assert_eq!(parse_verdict(""), None);
    }

    #[test]
    fn an_unreadable_escalation_keeps_the_screens_verdict() {
        let email = Email {
            id: "1".into(),
            sender: "a@b.test".into(),
            subject: "Hi".into(),
            body: "Text".into(),
            label: Label::Spam,
        };
        let triage = Triage {
            email: email.clone(),
            screening: screening(0.7),
            adjudication: Some(Adjudication {
                verdict: None,
                total_tokens: None,
                cost_usd: None,
            }),
        };
        assert_eq!(triage.verdict(), Label::Spam);
        assert!(triage.escalated());
        assert!(triage.correct());

        let overruled = Triage {
            adjudication: Some(Adjudication {
                verdict: Some(Label::Legitimate),
                total_tokens: None,
                cost_usd: None,
            }),
            ..triage
        };
        assert_eq!(overruled.verdict(), Label::Legitimate);
        assert!(!overruled.correct());
    }

    #[test]
    fn an_unescalated_email_answers_with_the_screen() {
        let triage = Triage {
            email: Email {
                id: "1".into(),
                sender: "a@b.test".into(),
                subject: "Hi".into(),
                body: "Text".into(),
                label: Label::Legitimate,
            },
            screening: screening(0.01),
            adjudication: None,
        };
        assert!(!triage.escalated());
        assert_eq!(triage.verdict(), Label::Legitimate);
        assert!(triage.correct());
    }
}
