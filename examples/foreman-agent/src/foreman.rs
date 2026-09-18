//! The supervisor half: nine narrow questions about one bounded observation.
//!
//! The worker is a generative model with a wide action space. The supervisor is
//! not: it answers the same nine yes/no questions every time, and it answers
//! them as probabilities rather than prose, so the decision stays in
//! [`policy`](crate::policy) rather than in a sentence this code has to parse.
//!
//! All nine ride one [`Classification`](everruns::Classification): questions in
//! a request are answered independently and in parallel, so asking nine costs
//! one round trip. That is what makes supervision cheap enough to run *while*
//! the worker works.

#[cfg(test)]
use std::sync::Mutex;
use std::time::Duration;

use everruns::{Classifier, ClassifierError};
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
/// interface to a classifier the way a schema is the interface to an API, and a
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
    /// The classification call failed or answered incompletely.
    Classifier(ClassifierError),
    /// The call did not return inside the configured budget.
    TimedOut(Duration),
    /// The observation could not be serialized into classifier state.
    State(serde_json::Error),
    /// A scripted supervisor was given no answers to replay.
    #[cfg(test)]
    EmptyScript,
}

impl std::fmt::Display for ForemanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Classifier(error) => write!(f, "{error}"),
            Self::TimedOut(budget) => {
                write!(f, "assessment exceeded {:.0}s", budget.as_secs_f64())
            }
            Self::State(error) => write!(f, "observation is not valid state: {error}"),
            #[cfg(test)]
            Self::EmptyScript => write!(f, "scripted supervisor has nothing to replay"),
        }
    }
}

impl std::error::Error for ForemanError {}

/// The supervisor.
///
/// [`Jev`](Self::Jev) is the real thing. [`Rehearsed`](Self::Rehearsed) derives
/// its numbers from the evidence instead of asking anything, which keeps the
/// offline demo the same run every time however the worker is scheduled.
/// `Scripted`, a test-only third, replays a fixed sequence so a run can be
/// driven down a path — a stuck worker, an unreachable service — that healthy
/// evidence never produces.
pub enum Foreman {
    /// A classifier answering the nine questions about live evidence.
    Jev {
        /// The classifier, and the service reaching it.
        classifier: Classifier,
        /// How long one assessment may take before the run treats it as absent.
        budget: Duration,
    },
    /// A fixed sequence, oldest first, holding on the last entry once spent.
    #[cfg(test)]
    Scripted(Mutex<Script>),
    /// A deterministic reading of the evidence itself.
    Rehearsed,
}

/// A fixed sequence of assessments and how far through it a run is.
#[cfg(test)]
pub struct Script {
    assessments: Vec<Assessment>,
    next: usize,
}

impl Foreman {
    /// Supervise through `classifier`, giving each assessment `budget`.
    pub fn jev(classifier: Classifier, budget: Duration) -> Self {
        Self::Jev { classifier, budget }
    }

    /// Replay `assessments` in order, holding on the last one once spent.
    #[cfg(test)]
    ///
    /// Holding rather than failing is what makes a scripted run robust: how
    /// many readings a run takes depends on how chatty the worker is, and the
    /// interesting part of a sequence is where it ends up.
    pub fn scripted(assessments: impl IntoIterator<Item = Assessment>) -> Self {
        Self::Scripted(Mutex::new(Script {
            assessments: assessments.into_iter().collect(),
            next: 0,
        }))
    }

    /// The model that answers, for display.
    pub fn model(&self) -> &str {
        match self {
            Self::Jev { .. } => crate::agent::FOREMAN_MODEL,
            #[cfg(test)]
            Self::Scripted(_) => "scripted",
            Self::Rehearsed => "rehearsed",
        }
    }

    /// Ask all nine questions about one observation.
    pub async fn assess(&self, observation: &Observation) -> Result<Assessment, ForemanError> {
        match self {
            Self::Jev { classifier, budget } => {
                let state = serde_json::to_value(observation).map_err(ForemanError::State)?;
                // One request, nine independent questions. Asking them one at a
                // time would cost nine round trips and still not be a snapshot:
                // the floor moves between calls.
                let mut classification = classifier.about(state);
                for dimension in &DIMENSIONS {
                    classification = classification.noul(dimension.id, dimension.question);
                }
                let answers = tokio::time::timeout(*budget, classification.send())
                    .await
                    .map_err(|_| ForemanError::TimedOut(*budget))?
                    .map_err(ForemanError::Classifier)?;

                let mut assessment = Assessment::default();
                for dimension in &DIMENSIONS {
                    let probability = answers
                        .probability(dimension.id)
                        .map_err(ForemanError::Classifier)?;
                    assessment.set(dimension.id, probability);
                }
                Ok(assessment)
            }
            #[cfg(test)]
            Self::Scripted(script) => {
                let mut script = script
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let last = script.assessments.len().checked_sub(1);
                let index = last
                    .map(|last| script.next.min(last))
                    .ok_or(ForemanError::EmptyScript)?;
                script.next = index + 1;
                script
                    .assessments
                    .get(index)
                    .copied()
                    .ok_or(ForemanError::EmptyScript)
            }
            Self::Rehearsed => Ok(rehearsed(observation)),
        }
    }
}

/// What a supervisor would plausibly say about this evidence.
///
/// Three readings, chosen by what is on the floor rather than by how many times
/// it has been asked: work under way, work finished but unchecked, and work an
/// independent pass has now looked at. The offline demo therefore walks the
/// same CONTINUE → START_VERIFIER → FINISH path whether the worker takes one
/// second or ten.
fn rehearsed(observation: &Observation) -> Assessment {
    let verified = observation
        .verification_results
        .iter()
        .any(|result| result.passed);
    let working = !observation.active_workers.is_empty();
    let touched = !observation.changed_files.is_empty();

    if verified {
        Assessment {
            implementation_complete: 0.98,
            tests_sufficient: 0.96,
            requirements_satisfied: 0.97,
            needs_verification: 0.04,
            ready_to_finish: 0.98,
            meaningful_progress: 0.98,
            worker_stuck: 0.00,
            work_off_track: 0.01,
            needs_human: 0.01,
        }
    } else if working {
        Assessment {
            implementation_complete: if touched { 0.72 } else { 0.31 },
            tests_sufficient: if touched { 0.28 } else { 0.10 },
            requirements_satisfied: if touched { 0.61 } else { 0.22 },
            needs_verification: if touched { 0.44 } else { 0.11 },
            ready_to_finish: if touched { 0.18 } else { 0.02 },
            meaningful_progress: if touched { 0.94 } else { 0.88 },
            worker_stuck: 0.03,
            work_off_track: 0.02,
            needs_human: 0.01,
        }
    } else {
        Assessment {
            implementation_complete: 0.96,
            tests_sufficient: 0.91,
            requirements_satisfied: 0.92,
            needs_verification: 0.93,
            ready_to_finish: 0.68,
            meaningful_progress: 0.95,
            worker_stuck: 0.01,
            work_off_track: 0.01,
            needs_human: 0.01,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let foreman = Foreman::jev(Classifier::simulated(0.42), Duration::from_secs(10));
        let assessment = foreman.assess(&observation()).await.unwrap();
        for dimension in &DIMENSIONS {
            assert_eq!(assessment.value(dimension.id), 0.42, "{}", dimension.id);
        }
    }

    #[tokio::test]
    async fn a_scripted_supervisor_replays_in_order_then_holds() {
        let first = Assessment {
            worker_stuck: 0.9,
            ..Assessment::default()
        };
        let second = Assessment {
            ready_to_finish: 0.95,
            ..Assessment::default()
        };
        let foreman = Foreman::scripted([first, second]);
        assert_eq!(foreman.assess(&observation()).await.unwrap(), first);
        assert_eq!(foreman.assess(&observation()).await.unwrap(), second);
        assert_eq!(foreman.assess(&observation()).await.unwrap(), second);
    }

    #[tokio::test]
    async fn an_empty_script_is_a_configuration_mistake() {
        let foreman = Foreman::scripted([]);
        assert!(matches!(
            foreman.assess(&observation()).await,
            Err(ForemanError::EmptyScript)
        ));
    }

    #[test]
    fn the_rehearsed_reading_follows_the_floor_not_the_call_count() {
        let mut working = Observation::sample();
        assert!(!working.active_workers.is_empty());
        let busy = rehearsed(&working);
        assert!(busy.ready_to_finish < 0.5);

        working.active_workers.clear();
        let idle = rehearsed(&working);
        assert!(idle.needs_verification > 0.65);
        assert!(idle.implementation_complete > 0.75);

        working
            .verification_results
            .push(crate::observation::VerificationResult {
                worker_id: "worker-2".into(),
                passed: true,
                summary: "checked".into(),
            });
        let verified = rehearsed(&working);
        assert!(verified.ready_to_finish > 0.85);
        assert!(verified.needs_verification < 0.65);
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
