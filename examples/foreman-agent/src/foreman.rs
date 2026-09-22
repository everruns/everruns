//! The supervisor half: nine narrow questions about one bounded observation.
//!
//! The worker is a generative model with a wide action space. The supervisor is
//! not: it answers the same nine yes/no questions every time, and it answers
//! them as probabilities rather than prose, so the decision stays in
//! [`policy`](crate::policy) rather than in a sentence this code has to parse.
//!
//! All nine ride one [`Decision`](everruns::Decision): questions in
//! a request are answered independently and in parallel, so asking nine costs
//! one round trip. That is what makes supervision cheap enough to run *while*
//! the worker works.

use std::time::Duration;

use everruns::{Decisions, DecisionsError};
use serde::Serialize;

use crate::observation::Observation;

/// Which half of the picture a dimension describes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lens {
    /// The job as a whole, independent of who is working on it right now.
    Job,
    /// The factory floor at this moment.
    Floor,
}

/// One supervised dimension: an id for this code, a question for the model.
pub struct Dimension {
    /// Labels the answer for this code. Never shown to the model.
    pub id: &'static str,
    /// The whole question. It has to read on its own — the id is not context.
    pub question: &'static str,
    /// Which half of the picture it describes.
    pub lens: Lens,
}

/// The nine dimensions, in the order they are reported.
///
/// Question wording is carried over from Foreman unchanged. Wording is the
/// interface to a decision service the way a schema is the interface to an API, and a
/// threshold calibrated against one phrasing says nothing about another.
pub const DIMENSIONS: [Dimension; 9] = [
    Dimension {
        id: "implementation_complete",
        question: "Is the implementation work required by the original job complete?",
        lens: Lens::Job,
    },
    Dimension {
        id: "tests_sufficient",
        question: "Does the work have sufficient relevant test coverage and passing verification?",
        lens: Lens::Job,
    },
    Dimension {
        id: "requirements_satisfied",
        question: "Does the current repository satisfy the original free-form job as a whole?",
        lens: Lens::Job,
    },
    Dimension {
        id: "needs_verification",
        question: "Does the current state warrant an independent verification pass before \
                   finishing?",
        lens: Lens::Job,
    },
    Dimension {
        id: "ready_to_finish",
        question: "Given all evidence, is the factory job ready to be declared complete?",
        lens: Lens::Job,
    },
    Dimension {
        id: "meaningful_progress",
        question: "Is the active or most recent worker making meaningful progress toward the job?",
        lens: Lens::Floor,
    },
    Dimension {
        id: "worker_stuck",
        question: "Does the active or most recent worker appear stuck, looping, or unable to \
                   advance?",
        lens: Lens::Floor,
    },
    Dimension {
        id: "work_off_track",
        question: "Is the current work drifting from the original job or making unrelated changes?",
        lens: Lens::Floor,
    },
    Dimension {
        id: "needs_human",
        question: "Does this situation require human judgment, credentials, clarification, or \
                   permission?",
        lens: Lens::Floor,
    },
];

/// The nine probabilities, normalized to `[0, 1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Assessment {
    /// Probability that required implementation work is complete.
    pub implementation_complete: f64,
    /// Probability that coverage and passing verification are sufficient.
    pub tests_sufficient: f64,
    /// Probability that the repository satisfies the free-form job as a whole.
    pub requirements_satisfied: f64,
    /// Probability that an independent verification pass is warranted.
    pub needs_verification: f64,
    /// Probability that the job should be considered complete.
    pub ready_to_finish: f64,
    /// Probability that the latest worker is advancing the job.
    pub meaningful_progress: f64,
    /// Probability that the worker is looping or unable to advance.
    pub worker_stuck: f64,
    /// Probability that work is drifting from the job.
    pub work_off_track: f64,
    /// Probability that a person is needed.
    pub needs_human: f64,
}

impl Assessment {
    /// Read one dimension by the id it was asked under.
    pub fn value(&self, id: &str) -> f64 {
        match id {
            "implementation_complete" => self.implementation_complete,
            "tests_sufficient" => self.tests_sufficient,
            "requirements_satisfied" => self.requirements_satisfied,
            "needs_verification" => self.needs_verification,
            "ready_to_finish" => self.ready_to_finish,
            "meaningful_progress" => self.meaningful_progress,
            "worker_stuck" => self.worker_stuck,
            "work_off_track" => self.work_off_track,
            "needs_human" => self.needs_human,
            _ => 0.0,
        }
    }

    fn set(&mut self, id: &str, value: f64) {
        // A probability outside [0, 1] is a service defect, not a decision
        // input: clamp rather than let it walk through a threshold comparison.
        let value = value.clamp(0.0, 1.0);
        match id {
            "implementation_complete" => self.implementation_complete = value,
            "tests_sufficient" => self.tests_sufficient = value,
            "requirements_satisfied" => self.requirements_satisfied = value,
            "needs_verification" => self.needs_verification = value,
            "ready_to_finish" => self.ready_to_finish = value,
            "meaningful_progress" => self.meaningful_progress = value,
            "worker_stuck" => self.worker_stuck = value,
            "work_off_track" => self.work_off_track = value,
            "needs_human" => self.needs_human = value,
            _ => {}
        }
    }
}

/// Why an assessment could not be made.
#[derive(Debug)]
pub enum ForemanError {
    /// The decision call failed or answered incompletely.
    Decisions(DecisionsError),
    /// The call did not return inside the configured budget.
    TimedOut(Duration),
    /// The observation could not be serialized into decisions state.
    State(serde_json::Error),
}

impl std::fmt::Display for ForemanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decisions(error) => write!(f, "{error}"),
            Self::TimedOut(budget) => {
                write!(f, "assessment exceeded {:.0}s", budget.as_secs_f64())
            }
            Self::State(error) => write!(f, "observation is not valid state: {error}"),
        }
    }
}

impl std::error::Error for ForemanError {}

/// The supervisor: a decision service, and how long one reading may take.
///
/// There is one of these, and it is always real. A run with no credentials is
/// not a different supervisor — it is the same code over a different
/// [`DecisionsService`](everruns::DecisionsService), which is the
/// Framework's own seam for that. See `demo/src/scripted.rs`.
pub struct Foreman {
    decisions: Decisions,
    budget: Duration,
}

impl Foreman {
    /// Supervise through `decisions`, giving each reading `budget`.
    pub fn new(decisions: Decisions, budget: Duration) -> Self {
        Self { decisions, budget }
    }

    /// Ask all nine questions about one observation.
    pub async fn assess(&self, observation: &Observation) -> Result<Assessment, ForemanError> {
        let state = serde_json::to_value(observation).map_err(ForemanError::State)?;
        // One request, nine independent questions. Asking them one at a time
        // would cost nine round trips and still not be a snapshot: the floor
        // moves between calls.
        let mut decision = self.decisions.about(state);
        for dimension in &DIMENSIONS {
            decision = decision.noul(dimension.id, dimension.question);
        }
        let answers = tokio::time::timeout(self.budget, decision.send())
            .await
            .map_err(|_| ForemanError::TimedOut(self.budget))?
            .map_err(ForemanError::Decisions)?;

        let mut assessment = Assessment::default();
        for dimension in &DIMENSIONS {
            let probability = answers
                .probability(dimension.id)
                .map_err(ForemanError::Decisions)?;
            assessment.set(dimension.id, probability);
        }
        Ok(assessment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use everruns::{AgentLoopError, DecisionOutcome, DecisionRequest, DecisionsService};

    /// A service that never answers, which is what a hung vendor looks like.
    struct Stalling;

    #[async_trait::async_trait]
    impl DecisionsService for Stalling {
        fn is_configured(&self) -> bool {
            true
        }

        async fn evaluate(
            &self,
            _request: DecisionRequest,
        ) -> Result<DecisionOutcome, AgentLoopError> {
            std::future::pending().await
        }
    }

    fn observation() -> Observation {
        Observation::sample()
    }

    #[test]
    fn every_dimension_round_trips_through_value() {
        let mut assessment = Assessment::default();
        for (index, dimension) in DIMENSIONS.iter().enumerate() {
            assessment.set(dimension.id, (index as f64 + 1.0) / 10.0);
        }
        for (index, dimension) in DIMENSIONS.iter().enumerate() {
            assert_eq!(assessment.value(dimension.id), (index as f64 + 1.0) / 10.0);
        }
    }

    #[test]
    fn out_of_range_probabilities_are_clamped() {
        let mut assessment = Assessment::default();
        assessment.set("worker_stuck", 1.4);
        assessment.set("needs_human", -0.2);
        assert_eq!(assessment.worker_stuck, 1.0);
        assert_eq!(assessment.needs_human, 0.0);
    }

    #[tokio::test]
    async fn a_classifier_answers_all_nine_in_one_request() {
        let foreman = Foreman::new(Decisions::simulated(0.42), Duration::from_secs(10));
        let assessment = foreman.assess(&observation()).await.unwrap();
        for dimension in &DIMENSIONS {
            assert_eq!(assessment.value(dimension.id), 0.42, "{}", dimension.id);
        }
    }

    #[tokio::test]
    async fn a_supervisor_that_cannot_answer_says_so_rather_than_guessing() {
        // A service that never returns is the failure a real deployment has;
        // the budget is what turns it into a decision instead of a hang.
        let foreman = Foreman::new(Decisions::new("slow", Stalling), Duration::from_millis(50));
        let error = foreman.assess(&observation()).await.unwrap_err();
        assert!(error.to_string().contains("exceeded"), "{error}");
    }

    #[test]
    fn the_nine_dimensions_split_into_job_and_floor() {
        assert_eq!(DIMENSIONS.iter().filter(|d| d.lens == Lens::Job).count(), 5);
        assert_eq!(
            DIMENSIONS.iter().filter(|d| d.lens == Lens::Floor).count(),
            4
        );
    }
}
