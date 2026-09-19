use crate::domains::budgets::BudgetService;
use crate::storage::StorageBackend;
use crate::storage::models::{CreateBudgetLedgerRow, CreateBudgetRow};
use everruns_provider::typed_id::BudgetId;
use std::sync::Arc;

#[tokio::test]
async fn test_check_budgets_exhausted_budget_returns_stop() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = BudgetService::new(db.clone());
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_1".into(),
            currency: "tokens".into(),
            limit: 100.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    db.create_budget_ledger_entry(CreateBudgetLedgerRow {
        budget_id: budget.id,
        amount: 100.0,
        meter_source: "llm_tokens".into(),
        ref_type: None,
        ref_id: None,
        session_id: None,
        description: None,
    })
    .await
    .unwrap();
    db.set_budget_status(budget.id, "exhausted").await.unwrap();

    let result = service
        .check_budgets_for_session(1, "session_1", None)
        .await;

    assert!(result.should_stop());
    assert_eq!(
        result.message.as_deref(),
        Some(
            "Budget exhausted. 100.00 tokens spent reached the 100.00 tokens limit. Increase the budget to continue."
        )
    );
    assert_eq!(result.error_code.as_deref(), Some("budget_exhausted"));
    assert_eq!(
        result
            .error_fields
            .as_ref()
            .and_then(|fields| fields.get("spent")),
        Some(&serde_json::json!(100.0))
    );
    assert_eq!(
        result
            .error_fields
            .as_ref()
            .and_then(|fields| fields.get("limit")),
        Some(&serde_json::json!(100.0))
    );
    assert_eq!(
        result
            .error_fields
            .as_ref()
            .and_then(|fields| fields.get("currency")),
        Some(&serde_json::json!("tokens"))
    );
    assert_eq!(
        result
            .error_fields
            .as_ref()
            .and_then(|fields| fields.get("budget_id")),
        Some(&serde_json::json!(
            BudgetId::from_uuid(budget.id).to_string()
        ))
    );
}
