//! Nine upstream questions, asked in one decision request.

use std::time::Duration;

use everruns::{Decisions, DecisionsError};
use serde::Serialize;

use crate::observation::Observation;

/// One supervised dimension: an id for this code, a question for the model.
pub struct Dimension {
    /// Labels the answer for this code. Never shown to the model.
    pub id: &'static str,
    /// The whole question. It has to read on its own — the id is not context.
    pub question: &'static str,
}

/// The nine dimensions, in the order they are reported.
pub const DIMENSIONS: [Dimension; 9] = [
    Dimension {
        id: "implementation_complete",
        question: "Is the implementation work required by the original job complete?",
    },
    Dimension {
        id: "tests_sufficient",
        question: "Does the work have sufficient relevant test coverage and passing verification?",
    },
    Dimension {
        id: "requirements_satisfied",
        question: "Does the current repository satisfy the original free-form job as a whole?",
    },
    Dimension {
        id: "needs_verification",
        question: "Does the current state warrant an independent verification pass before \
                   finishing?",
    },
    Dimension {
        id: "ready_to_finish",
        question: "Given all evidence, is the factory job ready to be declared complete?",
    },
    Dimension {
        id: "meaningful_progress",
        question: "Is the active or most recent worker making meaningful progress toward the job?",
    },
    Dimension {
        id: "worker_stuck",
        question: "Does the active or most recent worker appear stuck, looping, or unable to \
                   advance?",
    },
    Dimension {
        id: "work_off_track",
        question: "Is the current work drifting from the original job or making unrelated changes?",
    },
    Dimension {
        id: "needs_human",
        question: "Does this situation require human judgment, credentials, clarification, or \
                   permission?",
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
#[path = "../tests/unit/foreman.rs"]
mod tests;
