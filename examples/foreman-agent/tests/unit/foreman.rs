use super::*;

use everruns::{AgentLoopError, DecisionOutcome, DecisionRequest, DecisionsService};

/// A service that never answers, which is what a hung vendor looks like.
struct Stalling;

#[async_trait::async_trait]
impl DecisionsService for Stalling {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(&self, _request: DecisionRequest) -> Result<DecisionOutcome, AgentLoopError> {
        std::future::pending().await
    }
}

fn observation() -> Observation {
    crate::test_support::observation()
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
