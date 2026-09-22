//! Stability: alpha — may change without a major bump; see [`stability`](crate::stability).
//!
//! Direct decision, without an agent.
//!
//! Some work is not a prompt at all: it is a question with a typed answer.
//! *Is this claim supported by the source? How severe is this complaint? Which
//! queue does this ticket belong in?* A chat model answers those in prose that
//! your code then has to parse and trust. A decisions answers them as
//! numbers — a probability, a distribution over your options, a position along
//! your levels — so the decision stays in your code.
//!
//! This is the value-first surface for exactly that, and the deliberate
//! counterpart to [`Model::complete`](crate::Model::complete): same shape, a
//! different contract.
//!
//! | | [`Model`](crate::Model) | [`Decisions`] |
//! |---|---|---|
//! | you send | messages | state plus typed questions |
//! | you get back | text | calibrated numbers |
//! | decides the outcome | the model's words | your threshold, in your code |
//! | streams | yes | no — one round trip, nothing to stream |
//!
//! Start from a [`Decisions`]: [`Decisions::probability`] for the one-line case,
//! [`Decisions::about`] when the call asks more than one question or needs the
//! other primitives.
//!
//! ```
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns::Decisions;
//!
//! let decisions = Decisions::simulated(0.93);
//! let p = decisions
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
//! A decision keeps no history, runs no tools, and has no session — those stay
//! with [`Agent`](crate::Agent).
//!
//! The three primitives are System One's, and this API keeps their names rather
//! than inventing synonyms, so the vendor's own pages read as documentation for
//! these methods too: [Noul](https://docs.typesafe.ai/primitives/noul),
//! [Choice](https://docs.typesafe.ai/primitives/choice),
//! [Score](https://docs.typesafe.ai/primitives/score), and
//! [Confidence](https://docs.typesafe.ai/confidence) for what a `choice` or
//! `score` reports alongside its answer.

use std::fmt;
use std::sync::Arc;

use everruns_core::decisions::{
    DecisionAnswer, DecisionOutcome, DecisionQuestion, DecisionRequest, DecisionsService,
};
use everruns_provider::error::AgentLoopError;

/// Why a decision could not be made.
///
/// [`MissingService`](Self::MissingService), [`NoQuestions`](Self::NoQuestions)
/// and [`Unconfigured`](Self::Unconfigured) are configuration mistakes, caught
/// before any request leaves the process. [`Call`](Self::Call) carries the
/// service failure verbatim.
#[derive(Debug)]
#[non_exhaustive]
pub enum DecisionsError {
    /// No decisions service was attached.
    MissingService,
    /// The decision was sent with no questions.
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

impl fmt::Display for DecisionsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecisionsError::MissingService => {
                write!(
                    f,
                    "decisions has no service; use Decisions::new(model, service)"
                )
            }
            DecisionsError::NoQuestions => {
                write!(f, "decision has no questions; ask at least one")
            }
            DecisionsError::Unconfigured => write!(
                f,
                "decisions is not configured; set its deployment credential"
            ),
            DecisionsError::NoSuchAnswer(id) => write!(f, "no answer for question '{id}'"),
            DecisionsError::Call(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DecisionsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DecisionsError::Call(error) => Some(error),
            _ => None,
        }
    }
}

impl From<AgentLoopError> for DecisionsError {
    fn from(error: AgentLoopError) -> Self {
        DecisionsError::Call(error)
    }
}

/// A decisions, and the service used to reach it.
///
/// Cheap to clone and safe to share: one decisions can serve many concurrent
/// calls.
#[derive(Clone)]
pub struct Decisions {
    service: Option<Arc<dyn DecisionsService>>,
    model: Option<String>,
}

impl Decisions {
    /// Select a model and the service used to reach it.
    ///
    /// Reads as [`Model::new`](crate::Model::new) does, and for the same
    /// reason: the service is transport, the model is the thing that answers,
    /// and naming it is the caller's decision rather than a default they
    /// inherit without noticing. There will be other classifiers and other
    /// versions of this one, and a threshold calibrated against one version is
    /// not evidence about the next.
    ///
    /// The concrete service comes from an integration — for TypeSafe's System
    /// One, `everruns_integrations_typesafe::TypeSafeAI`.
    ///
    // `TypeSafeAI` is re-exported only under the `typesafe` feature, which is
    // not a default one. Compiling this example unconditionally made the bare
    // `cargo test -p everruns` fail for every contributor while CI, which runs
    // with the feature on, stayed green (EVE-1080). Marking the block `ignore`
    // outright would fix that by never compiling it again — including in the
    // build that can — so gate the fence and keep the example checked wherever
    // the type it needs actually exists.
    #[cfg_attr(feature = "typesafe", doc = "```no_run")]
    #[cfg_attr(not(feature = "typesafe"), doc = "```ignore")]
    /// # use everruns::{Decisions, TypeSafeAI};
    /// # fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let decisions = Decisions::new("jev-latest", TypeSafeAI::from_env()?);
    /// # let _ = decisions;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(model: impl Into<String>, service: impl DecisionsService + 'static) -> Self {
        Self {
            service: Some(Arc::new(service)),
            model: Some(model.into()),
        }
    }

    /// Select a model, reaching it through a service that is already shared.
    pub fn shared(model: impl Into<String>, service: Arc<dyn DecisionsService>) -> Self {
        Self {
            service: Some(service),
            model: Some(model.into()),
        }
    }

    /// A deterministic in-process decisions that answers every question the same
    /// way, with no credentials and no network.
    ///
    /// `probability` answers yes/no questions; choices and scores spread their
    /// mass evenly, so tests exercise the shape without asserting a vendor's
    /// numbers.
    pub fn simulated(probability: f64) -> Self {
        Self::new(SIMULATED_MODEL, SimulatedDecisionsService { probability })
    }

    /// Whether the underlying service is configured to answer at all.
    ///
    /// False means a deployment left its credential unset. Calls would fail
    /// with [`DecisionsError::Unconfigured`] rather than reaching anything.
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
    /// use everruns::Decisions;
    ///
    /// let p = Decisions::simulated(0.04)
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
    ) -> Result<f64, DecisionsError> {
        const ID: &str = "answer";
        self.about(state)
            .noul(ID, question)
            .send()
            .await?
            .probability(ID)
    }

    /// Describe a decision of `state`.
    pub fn about(&self, state: impl Into<serde_json::Value>) -> Decision {
        let mut request = DecisionRequest::new(state);
        request.model = self.model.clone();
        Decision {
            service: self.service.clone(),
            request,
        }
    }
}

impl fmt::Debug for Decisions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Decisions")
            .field(
                "service",
                &self.service.as_ref().map(|service| service.name()),
            )
            .finish()
    }
}

/// One decision call, described before it is sent.
///
/// Built with [`Decisions::about`]. Questions answer in parallel inside a single
/// request, so asking several is the cheap path, not the expensive one.
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use everruns::Decisions;
///
/// let answers = Decisions::simulated(0.8)
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
pub struct Decision {
    service: Option<Arc<dyn DecisionsService>>,
    request: DecisionRequest,
}

impl Decision {
    /// Reach the model through `service`, replacing the decisions's own.
    pub fn service(mut self, service: impl DecisionsService + 'static) -> Self {
        self.service = Some(Arc::new(service));
        self
    }

    /// Ask whether something holds, answered as the probability of yes.
    ///
    /// `instructions` must carry the whole question: `id` labels the answer for
    /// your code and never reaches the model. Read the number as a probability,
    /// not a grade — 0.5 means yes and no are near-equally likely, not
    /// "medium".
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let decisions = everruns::Decisions::simulated(0.96);
    /// let answers = decisions
    ///     .about("Wire the deposit today or the unit goes to another buyer.")
    ///     .noul("pressure", "Does this message apply time pressure?")
    ///     .send()
    ///     .await?;
    ///
    /// if answers.probability("pressure")? > 0.9 {
    ///     // your code decides; the model only measured
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// TypeSafe documents the primitive as
    /// [Noul](https://docs.typesafe.ai/primitives/noul).
    pub fn noul(self, id: impl Into<String>, instructions: impl Into<String>) -> Self {
        self.ask(id, DecisionQuestion::noul(instructions))
    }

    /// Ask a yes/no question, spelling out what each side means.
    ///
    /// Worth the extra words when "yes" is ambiguous — say what a yes covers
    /// and what a no covers, and the boundary stops being the model's guess.
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let decisions = everruns::Decisions::simulated(0.2);
    /// let answers = decisions
    ///     .about("Refunded on 3 Jan. Customer says it never arrived.")
    ///     .noul_between(
    ///         "resolved",
    ///         "Is this ticket resolved?",
    ///         "The refund reached the customer and they confirmed it",
    ///         "The refund is unconfirmed, disputed, or still in progress",
    ///     )
    ///     .send()
    ///     .await?;
    /// # let _ = answers.probability("resolved")?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// The `yes`/`no` criteria are what TypeSafe calls a Noul's criteria; see
    /// [Noul](https://docs.typesafe.ai/primitives/noul) and
    /// [Advanced: structure](https://docs.typesafe.ai/primitives/advanced) for
    /// giving them JSON structure rather than a sentence.
    pub fn noul_between(
        self,
        id: impl Into<String>,
        instructions: impl Into<String>,
        yes: impl Into<String>,
        no: impl Into<String>,
    ) -> Self {
        self.ask(
            id,
            DecisionQuestion::Noul {
                instructions: instructions.into(),
                yes: Some(yes.into()),
                no: Some(no.into()),
            },
        )
    }

    /// Ask for exactly one option from a set, with the distribution behind it.
    ///
    /// Needs at least two options; fewer is not a choice. [`Answers::selected`]
    /// takes the winner, and [`Answers::probability`] is not how you read one —
    /// the shape is a distribution, so ask for the answer you built.
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let decisions = everruns::Decisions::simulated(0.5);
    /// let answers = decisions
    ///     .about("My card was charged twice for one order.")
    ///     .choice(
    ///         "queue",
    ///         "Which team should handle this message?",
    ///         ["billing", "technical", "sales"],
    ///     )
    ///     .send()
    ///     .await?;
    ///
    /// let team: &str = answers.selected("queue")?;
    /// # let _ = team;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// TypeSafe documents the primitive as
    /// [Choice](https://docs.typesafe.ai/primitives/choice).
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
            DecisionQuestion::Choice {
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
    ///
    /// The answer is a weighted position, so it lands between levels;
    /// [`Answers::tail`] asks the more useful question — the probability of
    /// being *at least* a given level.
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let decisions = everruns::Decisions::simulated(0.5);
    /// let answers = decisions
    ///     .about("I've been on hold for two hours and my card was charged twice.")
    ///     .score(
    ///         "severity",
    ///         "How severe is the problem the writer describes?",
    ///         [
    ///             "A minor annoyance",
    ///             "A real problem with their account",
    ///             "Serious harm requiring immediate action",
    ///         ],
    ///     )
    ///     .send()
    ///     .await?;
    ///
    /// let position = answers.score("severity")?;        // e.g. 1.4 of 2
    /// let at_least_real = answers.tail("severity", 1)?; // P(level >= 1)
    /// # let _ = (position, at_least_real);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// TypeSafe documents the primitive as
    /// [Score](https://docs.typesafe.ai/primitives/score).
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
            DecisionQuestion::score(instructions, levels.into_iter().map(Into::into)),
        )
    }

    /// Attach a question built directly, for shapes the helpers do not cover.
    pub fn ask(mut self, id: impl Into<String>, question: DecisionQuestion) -> Self {
        self.request = self.request.ask(id, question);
        self
    }

    /// Ask a particular model for this call only.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.request.model = Some(model.into());
        self
    }

    /// Attach request metadata for host-side attribution.
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.request.metadata.insert(key.into(), value.into());
        self
    }

    /// Send the decision and wait for every answer.
    pub async fn send(self) -> Result<Answers, DecisionsError> {
        let (service, request) = self.into_request()?;
        Ok(Answers(service.evaluate(request).await?))
    }

    /// Validate the described call, keeping configuration mistakes off the
    /// wire.
    fn into_request(self) -> Result<(Arc<dyn DecisionsService>, DecisionRequest), DecisionsError> {
        let service = self.service.ok_or(DecisionsError::MissingService)?;
        if self.request.questions.is_empty() {
            return Err(DecisionsError::NoQuestions);
        }
        if !service.is_configured() {
            return Err(DecisionsError::Unconfigured);
        }
        Ok((service, self.request))
    }
}

impl fmt::Debug for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Decision")
            .field(
                "service",
                &self.service.as_ref().map(|service| service.name()),
            )
            .field("questions", &self.request.questions.len())
            .finish()
    }
}

/// Answers to one decision, keyed by the ids you asked under.
///
/// The accessors return the shape you asked for, so reading a `score` as a
/// probability is a typed mistake rather than a silent one.
#[derive(Debug, Clone)]
pub struct Answers(DecisionOutcome);

impl Answers {
    /// The model that answered, as the service resolved it.
    ///
    /// An alias resolves to a version here: ask for `jev-latest` and this
    /// reports the `jev-*` release that ran, which is the id to pin once a
    /// threshold is calibrated against it.
    ///
    /// ```
    /// # use everruns::Decisions;
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let answers = Decisions::simulated(0.9)
    ///     .about("Two hours on hold.")
    ///     .noul("urgent", "Does this convey urgency?")
    ///     .send()
    ///     .await?;
    /// println!("answered by {}", answers.model());
    /// # Ok(())
    /// # }
    /// ```
    pub fn model(&self) -> &str {
        &self.0.model
    }

    /// The probability of yes for a `noul` question.
    ///
    /// Near 0.5 means yes and no are near-equally likely — not "medium".
    pub fn probability(&self, id: &str) -> Result<f64, DecisionsError> {
        match self.answer(id)? {
            DecisionAnswer::Noul { probability } => Ok(*probability),
            _ => Err(DecisionsError::NoSuchAnswer(format!(
                "{id} was not asked as a yes/no question"
            ))),
        }
    }

    /// The selected option for a `choice` question.
    pub fn selected(&self, id: &str) -> Result<&str, DecisionsError> {
        match self.answer(id)? {
            DecisionAnswer::Choice { selected, .. } => Ok(selected.as_str()),
            _ => Err(DecisionsError::NoSuchAnswer(format!(
                "{id} was not asked as a choice"
            ))),
        }
    }

    /// The weighted position for a `score` question.
    ///
    /// For an "any serious hit" rule prefer [`tail`](Self::tail): an answer
    /// that is probably fine and possibly awful must not average into fine.
    pub fn score(&self, id: &str) -> Result<f64, DecisionsError> {
        match self.answer(id)? {
            DecisionAnswer::Score { score, .. } => Ok(*score),
            _ => Err(DecisionsError::NoSuchAnswer(format!(
                "{id} was not asked as a score"
            ))),
        }
    }

    /// The probability mass at or above `level` for a `score` question.
    ///
    /// The question worth asking for an escalation rule. A weighted score of
    /// 0.9 can hide a real chance of the worst level; the tail does not.
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let decisions = everruns::Decisions::simulated(0.5);
    /// let answers = decisions
    ///     .about("The deploy dropped the customers table.")
    ///     .score(
    ///         "severity",
    ///         "How severe is the incident described?",
    ///         ["Cosmetic", "Degraded service", "Data loss"],
    ///     )
    ///     .send()
    ///     .await?;
    ///
    /// // Not `score("severity")? > 1.5` — that averages a tail risk away.
    /// if answers.tail("severity", 2)? > 0.3 {
    ///     // page someone
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn tail(&self, id: &str, level: usize) -> Result<f64, DecisionsError> {
        match self.answer(id)? {
            DecisionAnswer::Score { probabilities, .. } => Ok(probabilities
                .iter()
                .filter(|(index, _)| **index >= level)
                .map(|(_, probability)| probability)
                .sum()),
            _ => Err(DecisionsError::NoSuchAnswer(format!(
                "{id} was not asked as a score"
            ))),
        }
    }

    /// The raw answer, for shapes the accessors do not cover.
    pub fn answer(&self, id: &str) -> Result<&DecisionAnswer, DecisionsError> {
        self.0
            .answers
            .get(id)
            .ok_or_else(|| DecisionsError::NoSuchAnswer(id.to_string()))
    }

    /// The whole outcome, including usage.
    ///
    /// The escape hatch for what the accessors do not surface — token counts,
    /// and the per-option or per-level distributions behind a `choice` or
    /// `score`, including the `confidence` TypeSafe documents under
    /// [Confidence](https://docs.typesafe.ai/confidence).
    pub fn outcome(&self) -> &DecisionOutcome {
        &self.0
    }
}

/// The model id [`Decisions::simulated`] reports, the way `Model::simulated`
/// reports `llmsim-model`: a stubbed answer must never pass for a real one.
const SIMULATED_MODEL: &str = "simulated";

/// Deterministic decisions for offline use; see [`Decisions::simulated`].
#[derive(Debug)]
struct SimulatedDecisionsService {
    probability: f64,
}

#[async_trait::async_trait]
impl DecisionsService for SimulatedDecisionsService {
    fn is_configured(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "SimulatedDecisionsService"
    }

    async fn evaluate(
        &self,
        request: DecisionRequest,
    ) -> everruns_provider::error::Result<DecisionOutcome> {
        // Names itself rather than a vendor model: `Answers::model` must never
        // let a stubbed answer pass for one a real decisions gave.
        let mut outcome = DecisionOutcome {
            model: SIMULATED_MODEL.to_string(),
            ..DecisionOutcome::default()
        };
        for (id, question) in &request.questions {
            let answer = match question {
                DecisionQuestion::Noul { .. } => DecisionAnswer::Noul {
                    probability: self.probability,
                },
                DecisionQuestion::Choice { options, .. } => {
                    let each = 1.0 / options.len().max(1) as f64;
                    DecisionAnswer::Choice {
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
                DecisionQuestion::Score { levels, .. } => {
                    let each = 1.0 / levels.len().max(1) as f64;
                    DecisionAnswer::Score {
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
        let p = Decisions::simulated(0.93)
            .probability("Does this convey urgency?", "Two hours on hold.")
            .await
            .expect("simulated decision succeeds");
        assert!((p - 0.93).abs() < 1e-9, "{p}");
    }

    #[tokio::test]
    async fn every_question_rides_one_request_and_keeps_its_id() {
        let answers = Decisions::simulated(0.8)
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
            .expect("simulated decision succeeds");

        assert!((answers.probability("urgent").expect("noul") - 0.8).abs() < 1e-9);
        assert_eq!(answers.selected("queue").expect("choice"), "billing");
        assert_eq!(answers.score("severity").expect("score"), 1.0);
        // Three even levels: the top two carry two thirds of the mass.
        let tail = answers.tail("severity", 1).expect("score tail");
        assert!((tail - 2.0 / 3.0).abs() < 1e-9, "{tail}");
    }

    #[tokio::test]
    async fn reading_an_answer_as_the_wrong_shape_is_an_error_not_a_number() {
        let answers = Decisions::simulated(0.5)
            .about("x")
            .score("rating", "How good?", ["Bad", "Good"])
            .send()
            .await
            .expect("simulated decision succeeds");
        assert!(matches!(
            answers.probability("rating"),
            Err(DecisionsError::NoSuchAnswer(_))
        ));
        assert!(matches!(
            answers.answer("missing"),
            Err(DecisionsError::NoSuchAnswer(_))
        ));
    }

    #[tokio::test]
    async fn configuration_mistakes_never_reach_the_service() {
        // No questions: nothing to ask.
        let empty = Decisions::simulated(0.5).about("x").send().await;
        assert!(matches!(empty, Err(DecisionsError::NoQuestions)));

        // No service at all.
        let decisions = Decisions {
            service: None,
            model: None,
        };
        let orphan = decisions.about("x").noul("q", "Is it?").send().await;
        assert!(matches!(orphan, Err(DecisionsError::MissingService)));

        // A service the deployment never configured answers nothing, and says
        // so rather than looking like a confident negative.
        let disabled = Decisions::new(
            "jev-latest",
            everruns_core::decisions::DisabledDecisionsService,
        );
        assert!(!disabled.is_configured());
        let unconfigured = disabled.about("x").noul("q", "Is it?").send().await;
        assert!(matches!(unconfigured, Err(DecisionsError::Unconfigured)));
    }

    #[test]
    fn debug_never_renders_a_credential_bearing_service() {
        let rendered = format!("{:?}", Decisions::simulated(0.5));
        assert!(rendered.contains("SimulatedDecisionsService"), "{rendered}");
    }
}

#[cfg(test)]
mod model_selection_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Records the model each request named, so the resolution order is
    /// observable rather than assumed.
    #[derive(Debug, Default)]
    struct RecordingService {
        seen: Mutex<Vec<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl DecisionsService for RecordingService {
        fn is_configured(&self) -> bool {
            true
        }

        async fn evaluate(
            &self,
            request: DecisionRequest,
        ) -> everruns_provider::error::Result<DecisionOutcome> {
            self.seen.lock().expect("lock").push(request.model.clone());
            Ok(DecisionOutcome::default())
        }
    }

    #[tokio::test]
    async fn the_classifiers_model_travels_and_a_call_outranks_it() {
        let service = Arc::new(RecordingService::default());
        let decisions = Decisions::shared("jev-1.13.0", service.clone());

        // No per-call model: the decisions's choice travels with the request.
        let _ = decisions.about("x").noul("q", "Does it hold?").send().await;
        // A per-call model wins over the decisions's.
        let _ = decisions
            .about("x")
            .noul("q", "Does it hold?")
            .model("jev-latest")
            .send()
            .await;

        let seen = service.seen.lock().expect("lock").clone();
        assert_eq!(
            seen,
            vec![
                Some("jev-1.13.0".to_string()),
                Some("jev-latest".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn a_request_built_without_a_classifier_can_still_leave_the_model_open() {
        // `Decisions` always names a model, but the wire contract does not
        // require one: the platform composes requests directly and leaves the
        // choice to the service, so the knob stays absent from every surface an
        // agent can write (THREAT[TM-LLM-037]).
        let service = RecordingService::default();
        let request = DecisionRequest::new(serde_json::json!("x"))
            .ask("q", DecisionQuestion::noul("Does it hold?"));
        assert_eq!(request.model, None);

        let _ = service.evaluate(request).await;

        assert_eq!(service.seen.lock().expect("lock").clone(), vec![None]);
    }

    #[tokio::test]
    async fn the_answer_reports_the_model_that_gave_it() {
        // The vendor resolves an alias to a version, so a caller reads back what
        // answered rather than what it asked for — the id to pin once a
        // threshold is calibrated. The stub names itself, so a simulated answer
        // can never pass for a real decisions's.
        let answers = Decisions::simulated(0.5)
            .about("x")
            .noul("q", "Does it hold?")
            .send()
            .await
            .expect("simulated answers");

        assert_eq!(answers.model(), "simulated");
    }
}
