// Budget metering of generations with declared cost components (EVE-1125).
//
// A managed-harness turn (the OpenAI Agents API backend) can bill tools and
// containers beyond its tokens, and can end without known usage. Priced
// components are debited; unknown ones stay explicit in the journal.

use super::tests::{create_session_with_owner, make_db};
use crate::domains::budgets::BudgetService;
use crate::storage::models::*;
use everruns_core::EventListener;
use everruns_core::events::{Event, EventContext, LlmGenerationData, TokenUsage};

#[tokio::test]
async fn test_unknown_usage_debits_priced_components_and_journals_the_unknown() {
    use everruns_core::events::LlmCostComponent;

    let db = make_db();
    let svc = BudgetService::new(db.clone());
    let session = create_session_with_owner(&db, 1, None, None).await;
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: session.org_id,
            subject_type: "session".into(),
            subject_id: session.id.to_string(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();

    // A cancelled managed turn: the provider never reported its usage, but it
    // ran two web searches and a hosted container.
    let mut data = LlmGenerationData::failure(
        vec![],
        vec![],
        "gpt-6-astra".into(),
        Some("openai".into()),
        "OpenAI Agents API turn cancelled".into(),
        None,
        None,
    )
    .with_cost_components(vec![
        LlmCostComponent::new(LlmCostComponent::MODEL_TOKENS, "gpt-6-astra", None, None),
        LlmCostComponent::new(
            LlmCostComponent::HOSTED_TOOL,
            "web_search_call",
            Some(2),
            Some(0.02),
        ),
        LlmCostComponent::new(LlmCostComponent::CONTAINER, "openai_hosted", None, None),
    ]);
    data.metadata.usage = None;
    svc.on_event(&Event::new(session.id, EventContext::empty(), data))
        .await;

    let entries = db
        .list_usage_ledger_for_budget(budget.id, 10, 0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1, "the priced component is debited");
    assert!((entries[0].amount - 0.02).abs() < 1e-9);
    let journal = db
        .get_usage_journal(entries[0].journal_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        journal.metadata["cost_unknown_components"],
        serde_json::json!(["model_tokens:gpt-6-astra", "container:openai_hosted"]),
        "unknown amounts stay explicit in the budget totals"
    );
}

#[test]
fn test_generation_without_usage_or_components_meters_nothing() {
    use crate::domains::budgets::service::GenerationMeter;
    let data = LlmGenerationData::failure(
        vec![],
        vec![],
        "gpt-6-astra".into(),
        None,
        "failed".into(),
        None,
        None,
    );
    assert!(GenerationMeter::from_metadata(&data.metadata).is_none());
    let data = LlmGenerationData::success(
        vec![],
        vec![],
        None,
        vec![],
        "gpt-6-astra".into(),
        None,
        Some(TokenUsage::new(10, 5)),
        None,
        None,
    );
    let meter = GenerationMeter::from_metadata(&data.metadata).unwrap();
    assert_eq!(meter.input_tokens, 10);
    assert!(meter.cost_unknown_components.is_empty());
}
