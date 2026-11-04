//! Live smoke tests for decision-model discovery on System One hosts.
//!
//! TypeSafe and OpenRouter publish decision-model catalogs; these check that a
//! sync would list them, and that a listed id is one the host answers.

use everruns_contracts::decisions::{DecisionQuestion, DecisionRequest};

#[tokio::test]
async fn openrouter_lists_decision_models_it_answers() {
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .expect("OPENROUTER_API_KEY must be set for the live smoke test");
    let provider = everruns_drivers::openrouter::provider("openrouter", api_key);

    let models = provider
        .list_models()
        .await
        .expect("list_models should succeed")
        .expect("OpenRouter publishes a catalog");
    let decisions: Vec<_> = models
        .iter()
        .filter(|m| m.capabilities.iter().any(|c| c == "decisions"))
        .collect();
    assert!(
        decisions.len() > 5,
        "expected several decision models, got {}",
        decisions.len()
    );
    assert!(
        decisions.iter().all(|m| m
            .discovered_profile
            .as_ref()
            .is_some_and(|p| p.decisions.is_some())),
        "every decision model carries a decision profile"
    );
    // Chat discovery still answers alongside.
    assert!(
        models
            .iter()
            .any(|m| m.capabilities.iter().any(|c| c == "chat"))
    );

    // The registry's `jev-latest` row must reach a model OpenRouter serves.
    let outcome = provider
        .evaluate_decisions(
            DecisionRequest::new("The sky is clear and blue today.")
                .model("jev-latest")
                .ask("clear", DecisionQuestion::noul("Is the weather clear?")),
        )
        .await
        .expect("jev-latest answers through OpenRouter");
    assert!(outcome.answers.contains_key("clear"));
}

#[tokio::test]
async fn typesafe_lists_its_decision_models() {
    let api_key = std::env::var("TYPESAFE_API_KEY")
        .expect("TYPESAFE_API_KEY must be set for the live smoke test");
    let provider = everruns_drivers::typesafe::provider("typesafe", api_key);

    let models = provider
        .list_models()
        .await
        .expect("list_models should succeed")
        .expect("TypeSafe publishes a catalog");
    assert!(
        models.iter().any(|m| m.model_id == "jev-latest"),
        "jev-latest is listed: {:?}",
        models.iter().map(|m| &m.model_id).collect::<Vec<_>>()
    );
    assert!(models.iter().all(|m| m.capabilities == ["decisions"]));
}
