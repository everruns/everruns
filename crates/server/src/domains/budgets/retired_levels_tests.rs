// Retired budget subject levels (EVE-1129).
//
// The `app` level is gone, converted onto the agent — a column rather than the
// `app:` tag it used to come from.
//
// `app_channel` is still read from a tag, and deliberately so: migration 138
// moved App webhooks onto `agent_triggers`, moved their budgets back from
// `agent_endpoint` to `app_channel`, and deleted the endpoint rows those
// budgets had been keyed on. For a webhook trigger the tag is the only
// identifier left, so retiring the level would delete a live cap rather than
// move it (EVE-1138). The prefix is reserved, so an org member cannot forge it.

use crate::domains::budgets::BudgetService;
use crate::storage::StorageBackend;
use crate::storage::models::*;
use everruns_provider::typed_id::PrincipalId;
use std::sync::Arc;

fn make_service() -> (BudgetService, Arc<StorageBackend>) {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = BudgetService::new(db.clone());
    (svc, db)
}

async fn create_session_with_tags(
    db: &Arc<StorageBackend>,
    org_id: i64,
    tags: Vec<String>,
) -> SessionRow {
    db.create_session(CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id,
        app_id: None,
        endpoint_id: None,
        harness_id: None,
        agent_id: None,
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id: PrincipalId::new(),
        resolved_owner_user_id: None,
        title: Some("Retired level budget test session".into()),
        locale: None,
        tags,
        model_id: None,
        capabilities: serde_json::json!({}),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!([]),
        system_prompt: None,
        initial_files: serde_json::json!({}),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn the_app_tag_no_longer_contributes_a_budget_subject() {
    let (svc, db) = make_service();
    let session = create_session_with_tags(
        &db,
        1,
        vec![
            "app:app_for_budget_test".to_string(),
            "app_channel:appchan_for_budget_test".to_string(),
            "slack:app:app_legacy".to_string(),
            "ag_ui:app:app_ag_ui".to_string(),
        ],
    )
    .await;

    for (subject_type, subject_id) in [
        ("app", "app_for_budget_test"),
        ("app", "app_legacy"),
        ("app", "app_ag_ui"),
        ("app_channel", "appchan_for_budget_test"),
    ] {
        db.create_budget(CreateBudgetRow {
            org_id: session.org_id,
            subject_type: subject_type.into(),
            subject_id: subject_id.into(),
            currency: "usd".into(),
            limit: 5.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    }

    let budgets = svc
        .list_budgets_for_session_hierarchy(session.org_id, &session.id.to_string(), None)
        .await;
    let subjects: Vec<&str> = budgets.iter().map(|b| b.subject_type.as_str()).collect();
    assert!(
        !subjects.contains(&"app"),
        "the retired `app` level must not resolve, from any of its three tag \
         spellings: {subjects:?}"
    );
    assert!(
        subjects.contains(&"app_channel"),
        "`app_channel` still binds — a webhook trigger has no other \
         attribution: {subjects:?}"
    );
}
