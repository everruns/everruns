//! Direct classification, without an agent.
//!
//! Some work is not a prompt at all: it is a question with a typed answer.
//! *Is this claim supported by the source? How severe is this complaint? Which
//! queue does this ticket belong in?* A chat model answers those in prose that
//! your code then has to parse and trust. A classifier answers them as
//! numbers — a probability, a distribution over your options, a position along
//! your levels — so the decision stays in your code.
//!
//! This is the value-first surface for exactly that, and the deliberate
//! counterpart to [`Model::complete`](crate::Model::complete): same shape, a
//! different contract.
//!
//! | | [`Model`](crate::Model) | [`Classifier`] |
//! |---|---|---|
//! | you send | messages | state plus typed questions |
//! | you get back | text | calibrated numbers |
//! | decides the outcome | the model's words | your threshold, in your code |
//! | streams | yes | no — one round trip, nothing to stream |
//!
//! Start from a [`Classifier`]: [`Classifier::probability`] for the one-line case,
//! [`Classifier::about`] when the call asks more than one question or needs the
//! other primitives.
//!
//! ```
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns::Classifier;
//!
//! let classifier = Classifier::simulated(0.93);
//! let p = classifier
//!     .probability("Does this convey urgency?", "I've been on hold for two hours.")
//!     .await?;
//! assert!(p > 0.9);
//! # Ok(())
//! # }
//! ```
//!
//! Questions are answered in parallel within one request, so asking five costs
//! one round trip. Ids are yours: they label answers for your code and are
//! never shown to the model, so every question must read on its own.
//!
//! A classification keeps no history, runs no tools, and has no session — those stay
//! with [`Agent`](crate::Agent).

use std::fmt;
use std::sync::Arc;

use everruns_core::classifier::{
    ClassificationAnswer, ClassificationOutcome, ClassificationQuestion, ClassificationRequest,
    ClassifierService,
};
use everruns_provider::error::AgentLoopError;

/// Why a classification could not be made.
///
/// [`MissingService`](Self::MissingService), [`NoQuestions`](Self::NoQuestions)
/// and [`Unconfigured`](Self::Unconfigured) are configuration mistakes, caught
/// before any request leaves the process. [`Call`](Self::Call) carries the
/// service failure verbatim.
#[derive(Debug)]
#[non_exhaustive]
pub enum ClassifierError {
    /// No classifier service was attached.
    MissingService,
    /// The classification was sent with no questions.
    NoQuestions,
    /// The service exists but the deployment never configured it, so it would
    /// answer nothing.
    ///
    /// Distinct from [`Call`](Self::Call): nothing was attempted. A guardrail
    /// treats this as fail-open; a direct caller usually wants to know.
    Unconfigured,
    /// The question id asked for is not in the answers.
    NoSuchAnswer(String),
    /// The service call failed.
    Call(AgentLoopError),
}

impl fmt::Display for ClassifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClassifierError::MissingService => {
                write!(f, "classifier has no service; use Classifier::new(service)")
            }
            ClassifierError::NoQuestions => {
                write!(f, "classification has no questions; ask at least one")
            }
            ClassifierError::Unconfigured => write!(
                f,
                "classifier is not configured; set its deployment credential"
            ),
            ClassifierError::NoSuchAnswer(id) => write!(f, "no answer for question '{id}'"),
            ClassifierError::Call(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ClassifierError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClassifierError::Call(error) => Some(error),
            _ => None,
        }
    }
}

impl From<AgentLoopError> for ClassifierError {
    fn from(error: AgentLoopError) -> Self {
        ClassifierError::Call(error)
    }
}

/// A classifier, and the service used to reach it.
///
/// Cheap to clone and safe to share: one classifier can serve many concurrent
/// calls.
#[derive(Clone)]
pub struct Classifier {
    service: Option<Arc<dyn ClassifierService>>,
}

impl Classifier {
    /// Reach a classifier through `service`.
    ///
    /// The concrete service comes from an integration — for TypeSafe's System
    /// One, `everruns_integrations_typesafe::TypeSafeClassifier` — the
    /// same way a [`Model`](crate::Model) takes its provider from a driver.
    pub fn new(service: impl ClassifierService + 'static) -> Self {
        Self {
            service: Some(Arc::new(service)),
        }
    }

    /// Reach a classifier through a service that is already shared.
    pub fn shared(service: Arc<dyn ClassifierService>) -> Self {
        Self {
            service: Some(service),
        }
    }

    /// A deterministic in-process classifier that answers every question the same
    /// way, with no credentials and no network.
    ///
    /// `probability` answers yes/no questions; choices and scores spread their
    /// mass evenly, so tests exercise the shape without asserting a vendor's
    /// numbers.
    pub fn simulated(probability: f64) -> Self {
        Self::new(SimulatedClassifierService { probability })
    }

    /// Whether the underlying service is configured to answer at all.
    ///
    /// False means a deployment left its credential unset. Calls would fail
    /// with [`ClassifierError::Unconfigured`] rather than reaching anything.
    pub fn is_configured(&self) -> bool {
        self.service
            .as_ref()
            .is_some_and(|service| service.is_configured())
    }

    /// Ask one yes/no question about `state` and take the probability of yes.
    ///
    /// The whole one-shot path. Use [`about`](Self::about) when the call asks
    /// more than one question, or needs a choice or a score.
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use everruns::Classifier;
    ///
    /// let p = Classifier::simulated(0.04)
    ///     .probability("Is this spam?", "Standup moved to 10am.")
    ///     .await?;
    /// assert!(p < 0.1);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn probability(
        &self,
        question: impl Into<String>,
        state: impl Into<serde_json::Value>,
    ) -> Result<f64, ClassifierError> {
        const ID: &str = "answer";
        self.about(state)
            .noul(ID, question)
            .send()
            .await?
            .probability(ID)
    }

    /// Describe a classification of `state`.
    pub fn about(&self, state: impl Into<serde_json::Value>) -> Classification {
        Classification {
            service: self.service.clone(),
            request: ClassificationRequest::new(state),
        }
    }
}

impl fmt::Debug for Classifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Classifier")
            .field(
                "service",
                &self.service.as_ref().map(|service| service.name()),
            )
            .finish()
    }
}

/// One classification call, described before it is sent.
///
/// Built with [`Classifier::about`]. Questions answer in parallel inside a single
/// request, so asking several is the cheap path, not the expensive one.
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use everruns::Classifier;
///
/// let answers = Classifier::simulated(0.8)
///     .about("I've been on hold for two hours and nobody can tell me why.")
///     .noul("urgent", "Does this convey urgency?")
///     .score(
///         "severity",
///         "How severe is the problem described?",
///         ["Minor annoyance", "Real problem", "Serious harm"],
///     )
///     .choice(
///         "queue",
///         "Which team should handle this?",
///         ["billing", "technical", "sales"],
///     )
///     .send()
///     .await?;
///
/// assert!(answers.probability("urgent")? > 0.5);
/// # Ok(())
/// # }
/// ```
pub struct Classification {
    service: Option<Arc<dyn ClassifierService>>,
    request: ClassificationRequest,
}

impl Classification {
    /// Reach the model through `service`, replacing the classifier's own.
    pub fn service(mut self, service: impl ClassifierService + 'static) -> Self {
        self.service = Some(Arc::new(service));
        self
    }

    /// Ask whether something holds, answered as the probability of yes.
    ///
    /// `instructions` must carry the whole question: `id` labels the answer for
    /// your code and never reaches the model.
    pub fn noul(self, id: impl Into<String>, instructions: impl Into<String>) -> Self {
        self.ask(id, ClassificationQuestion::noul(instructions))
    }

    /// Ask a yes/no question, spelling out what each side means.
    ///
    /// Worth the extra words when "yes" is ambiguous — say what a yes covers
    /// and what a no covers, and the boundary stops being the model's guess.
    pub fn noul_between(
        self,
        id: impl Into<String>,
        instructions: impl Into<String>,
        yes: impl Into<String>,
        no: impl Into<String>,
    ) -> Self {
        self.ask(
            id,
            ClassificationQuestion::Noul {
                instructions: instructions.into(),
                yes: Some(yes.into()),
                no: Some(no.into()),
            },
        )
    }

    /// Ask for exactly one option from a set, with the distribution behind it.
    ///
    /// Needs at least two options; fewer is not a choice.
    pub fn choice<O, S>(
        self,
        id: impl Into<String>,
        instructions: impl Into<String>,
        options: O,
    ) -> Self
    where
        O: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.ask(
            id,
            ClassificationQuestion::Choice {
                instructions: instructions.into(),
                options: options
                    .into_iter()
                    .map(|option| (option.into(), None))
                    .collect(),
            },
        )
    }

    /// Ask for a position along ordered levels, lowest first.
    ///
    /// Each level describes a concrete situation, not a grade: "Minor
    /// annoyance" reads on its own where "2 out of 5" does not. Needs at least
    /// two levels.
    pub fn score<L, S>(
        self,
        id: impl Into<String>,
        instructions: impl Into<String>,
        levels: L,
    ) -> Self
    where
        L: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.ask(
            id,
            ClassificationQuestion::score(instructions, levels.into_iter().map(Into::into)),
        )
    }

    /// Attach a question built directly, for shapes the helpers do not cover.
    pub fn ask(mut self, id: impl Into<String>, question: ClassificationQuestion) -> Self {
        self.request = self.request.ask(id, question);
        self
    }

    /// Attach request metadata for host-side attribution.
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.request.metadata.insert(key.into(), value.into());
        self
    }

    /// Send the classification and wait for every answer.
    pub async fn send(self) -> Result<Answers, ClassifierError> {
        let (service, request) = self.into_request()?;
        Ok(Answers(service.evaluate(request).await?))
    }

    /// Validate the described call, keeping configuration mistakes off the
    /// wire.
    fn into_request(
        self,
    ) -> Result<(Arc<dyn ClassifierService>, ClassificationRequest), ClassifierError> {
        let service = self.service.ok_or(ClassifierError::MissingService)?;
        if self.request.questions.is_empty() {
            return Err(ClassifierError::NoQuestions);
        }
        if !service.is_configured() {
            return Err(ClassifierError::Unconfigured);
        }
        Ok((service, self.request))
    }
}

impl fmt::Debug for Classification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Classification")
            .field(
                "service",
                &self.service.as_ref().map(|service| service.name()),
            )
            .field("questions", &self.request.questions.len())
            .finish()
    }
}

/// Answers to one classification, keyed by the ids you asked under.
///
/// The accessors return the shape you asked for, so reading a `score` as a
/// probability is a typed mistake rather than a silent one.
#[derive(Debug, Clone)]
pub struct Answers(ClassificationOutcome);

impl Answers {
    /// The probability of yes for a `noul` question.
    ///
    /// Near 0.5 means yes and no are near-equally likely — not "medium".
    pub fn probability(&self, id: &str) -> Result<f64, ClassifierError> {
        match self.answer(id)? {
            ClassificationAnswer::Noul { probability } => Ok(*probability),
            _ => Err(ClassifierError::NoSuchAnswer(format!(
                "{id} was not asked as a yes/no question"
            ))),
        }
    }

    /// The selected option for a `choice` question.
    pub fn selected(&self, id: &str) -> Result<&str, ClassifierError> {
        match self.answer(id)? {
            ClassificationAnswer::Choice { selected, .. } => Ok(selected.as_str()),
            _ => Err(ClassifierError::NoSuchAnswer(format!(
                "{id} was not asked as a choice"
            ))),
        }
    }

    /// The weighted position for a `score` question.
    ///
    /// For an "any serious hit" rule prefer [`tail`](Self::tail): an answer
    /// that is probably fine and possibly awful must not average into fine.
    pub fn score(&self, id: &str) -> Result<f64, ClassifierError> {
        match self.answer(id)? {
            ClassificationAnswer::Score { score, .. } => Ok(*score),
            _ => Err(ClassifierError::NoSuchAnswer(format!(
                "{id} was not asked as a score"
            ))),
        }
    }

    /// The probability mass at or above `level` for a `score` question.
    pub fn tail(&self, id: &str, level: usize) -> Result<f64, ClassifierError> {
        match self.answer(id)? {
            ClassificationAnswer::Score { probabilities, .. } => Ok(probabilities
                .iter()
                .filter(|(index, _)| **index >= level)
                .map(|(_, probability)| probability)
                .sum()),
            _ => Err(ClassifierError::NoSuchAnswer(format!(
                "{id} was not asked as a score"
            ))),
        }
    }

    /// The raw answer, for shapes the accessors do not cover.
    pub fn answer(&self, id: &str) -> Result<&ClassificationAnswer, ClassifierError> {
        self.0
            .answers
            .get(id)
            .ok_or_else(|| ClassifierError::NoSuchAnswer(id.to_string()))
    }

    /// The whole outcome, including usage.
    pub fn outcome(&self) -> &ClassificationOutcome {
        &self.0
    }
}

/// Deterministic classifications for offline use; see [`Classifier::simulated`].
#[derive(Debug)]
struct SimulatedClassifierService {
    probability: f64,
}

#[async_trait::async_trait]
impl ClassifierService for SimulatedClassifierService {
    fn is_configured(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "SimulatedClassifierService"
    }

    async fn evaluate(
        &self,
        request: ClassificationRequest,
    ) -> everruns_provider::error::Result<ClassificationOutcome> {
        let mut outcome = ClassificationOutcome::default();
        for (id, question) in &request.questions {
            let answer = match question {
                ClassificationQuestion::Noul { .. } => ClassificationAnswer::Noul {
                    probability: self.probability,
                },
                ClassificationQuestion::Choice { options, .. } => {
                    let each = 1.0 / options.len().max(1) as f64;
                    ClassificationAnswer::Choice {
                        selected: options
                            .first()
                            .map(|(name, _)| name.clone())
                            .unwrap_or_default(),
                        probabilities: options
                            .iter()
                            .map(|(name, _)| (name.clone(), each))
                            .collect(),
                        confidence: each,
                    }
                }
                ClassificationQuestion::Score { levels, .. } => {
                    let each = 1.0 / levels.len().max(1) as f64;
                    ClassificationAnswer::Score {
                        score: (levels.len().saturating_sub(1)) as f64 / 2.0,
                        probabilities: (0..levels.len()).map(|index| (index, each)).collect(),
                        confidence: each,
                    }
                }
            };
            outcome.answers.insert(id.clone(), answer);
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probability_answers_the_one_line_case() {
        let p = Classifier::simulated(0.93)
            .probability("Does this convey urgency?", "Two hours on hold.")
            .await
            .expect("simulated classification succeeds");
        assert!((p - 0.93).abs() < 1e-9, "{p}");
    }

    #[tokio::test]
    async fn every_question_rides_one_request_and_keeps_its_id() {
        let answers = Classifier::simulated(0.8)
            .about("I've been on hold for two hours.")
            .noul("urgent", "Does this convey urgency?")
            .score(
                "severity",
                "How severe is the problem?",
                ["Minor", "Real", "Serious"],
            )
            .choice("queue", "Which team?", ["billing", "technical"])
            .send()
            .await
            .expect("simulated classification succeeds");

        assert!((answers.probability("urgent").expect("noul") - 0.8).abs() < 1e-9);
        assert_eq!(answers.selected("queue").expect("choice"), "billing");
        assert_eq!(answers.score("severity").expect("score"), 1.0);
        // Three even levels: the top two carry two thirds of the mass.
        let tail = answers.tail("severity", 1).expect("score tail");
        assert!((tail - 2.0 / 3.0).abs() < 1e-9, "{tail}");
    }

    #[tokio::test]
    async fn reading_an_answer_as_the_wrong_shape_is_an_error_not_a_number() {
        let answers = Classifier::simulated(0.5)
            .about("x")
            .score("rating", "How good?", ["Bad", "Good"])
            .send()
            .await
            .expect("simulated classification succeeds");
        assert!(matches!(
            answers.probability("rating"),
            Err(ClassifierError::NoSuchAnswer(_))
        ));
        assert!(matches!(
            answers.answer("missing"),
            Err(ClassifierError::NoSuchAnswer(_))
        ));
    }

    #[tokio::test]
    async fn configuration_mistakes_never_reach_the_service() {
        // No questions: nothing to ask.
        let empty = Classifier::simulated(0.5).about("x").send().await;
        assert!(matches!(empty, Err(ClassifierError::NoQuestions)));

        // No service at all.
        let classifier = Classifier { service: None };
        let orphan = classifier.about("x").noul("q", "Is it?").send().await;
        assert!(matches!(orphan, Err(ClassifierError::MissingService)));

        // A service the deployment never configured answers nothing, and says
        // so rather than looking like a confident negative.
        let disabled = Classifier::new(everruns_core::classifier::DisabledClassifierService);
        assert!(!disabled.is_configured());
        let unconfigured = disabled.about("x").noul("q", "Is it?").send().await;
        assert!(matches!(unconfigured, Err(ClassifierError::Unconfigured)));
    }

    #[test]
    fn debug_never_renders_a_credential_bearing_service() {
        let rendered = format!("{:?}", Classifier::simulated(0.5));
        assert!(
            rendered.contains("SimulatedClassifierService"),
            "{rendered}"
        );
    }
}
