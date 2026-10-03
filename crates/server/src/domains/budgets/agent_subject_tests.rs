// Agent-subject budget binding (EVE-1136).
//
// Budgets are keyed by the identifiers the API exposes. For the `agent`
// subject that is `agents.public_id` — not `agents.id`, which is a separately
// generated UUID that a typed `AgentId` off a session row renders. The
// resolver used to look up `'agent_' || agents.id`, a subject no caller could
// ever have created a budget for, so every agent-scoped budget was stored and
// displayed but never evaluated.

use crate::domains::budgets::BudgetService;
use crate::storage::StorageBackend;
use crate::storage::models::*;
use everruns_contracts::typed_id::{AgentId, PrincipalId};
use everruns_core::EventListener;
use everruns_core::events::{Event, EventContext, LlmGenerationData, TokenUsage};
use std::sync::Arc;
use uuid::Uuid;

fn make_service() -> (BudgetService, Arc<StorageBackend>) {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = BudgetService::new(db.clone());
    (svc, db)
}

async fn create_session_with_owner(
    db: &Arc<StorageBackend>,
    org_id: i64,
    agent_id: Option<AgentId>,
    resolved_owner_user_id: Option<Uuid>,
) -> SessionRow {
    db.create_session(CreateSessionRow {
        playground_user_id: None,
        source: crate::records::SessionSource::Api,
        workspace_id: None,
        org_id,
        app_id: None,
        endpoint_id: None,
        trigger_id: None,
        harness_id: None,
        agent_id,
        agent_version_id: None,
        agent_config_hash: None,
        virtual_user_id: None,
        owner_principal_id: PrincipalId::new(),
        resolved_owner_user_id,
        title: Some("Agent subject budget test session".into()),
        locale: None,
        tags: vec![],
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

/// A real agent row, with `id` and `public_id` independent as production
/// leaves them. Shared with `tests.rs`: the hierarchy test there used to key
/// its budget on a synthetic `AgentId::new()` that existed in no table, which
/// is why the agent level's breakage went unnoticed for so long.
pub(super) async fn seed_agent(db: &Arc<StorageBackend>, name: &str) -> AgentRow {
    db.create_agent(
        1,
        CreateAgentRow {
            public_id: AgentId::new().to_string(),
            name: name.into(),
            display_name: None,
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: serde_json::json!([]),
            system_prompt: String::new(),
            default_model_id: None,
            harness_id: everruns_contracts::typed_id::HarnessId::new(),
            tags: vec![],
            initial_files: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            environments: None,
            is_built_in: false,
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn agent_budget_keyed_by_public_id_binds_to_the_agents_sessions() {
    let (svc, db) = make_service();
    let agent = seed_agent(&db, "public-id-budget-agent").await;
    // The two identifiers really are different values; the whole bug lives in
    // the gap between them.
    assert_ne!(
        agent.public_id,
        agent.id.to_string(),
        "fixture must exercise the id/public_id gap"
    );
    let session = create_session_with_owner(&db, 1, Some(agent.id), None).await;

    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            // Exactly what POST /v1/budgets stores: the agent's API-facing id.
            subject_id: agent.public_id.clone(),
            subject_type: "agent".into(),
            currency: "tokens".into(),
            limit: 150.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();

    let hierarchy = svc
        .list_budgets_for_session_hierarchy(1, &session.id.to_string(), None)
        .await;
    assert_eq!(
        hierarchy.iter().filter(|b| b.id == budget.id).count(),
        1,
        "agent budget keyed by public_id must appear in the session hierarchy"
    );
}

#[tokio::test]
async fn agent_budget_keyed_by_public_id_exhausts_and_stops_the_session() {
    let (svc, db) = make_service();
    let agent = seed_agent(&db, "stopping-budget-agent").await;
    let session = create_session_with_owner(&db, 1, Some(agent.id), None).await;

    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_id: agent.public_id.clone(),
            subject_type: "agent".into(),
            currency: "tokens".into(),
            limit: 150.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();

    let data = LlmGenerationData::success(
        vec![],
        vec![],
        Some("agent session spent its budget".into()),
        vec![],
        "gpt-5.4-mini".into(),
        Some("openai".into()),
        Some(TokenUsage::new(100, 50)),
        None,
        None,
    );
    svc.on_event(&Event::new(session.id, EventContext::empty(), data))
        .await;

    let updated = db.get_budget(1, budget.id).await.unwrap().unwrap();
    assert_eq!(updated.balance, 0.0, "the agent budget must be debited");
    assert_eq!(updated.status, "exhausted");

    let result = svc
        .check_budgets_for_session(1, &session.id.to_string(), None)
        .await;
    assert!(result.should_stop(), "an exhausted agent budget must stop");
    assert_eq!(
        result.budget_id,
        Some(everruns_contracts::typed_id::BudgetId::from_uuid(budget.id))
    );
}

#[tokio::test]
async fn agent_budget_binds_through_the_agent_id_override_path() {
    let (svc, db) = make_service();
    let agent = seed_agent(&db, "override-budget-agent").await;
    // A session with no agent of its own — the worker supplies the agent, as
    // DirectBudgetChecker and the gRPC policy path both do.
    let session = create_session_with_owner(&db, 1, None, None).await;

    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_id: agent.public_id.clone(),
            subject_type: "agent".into(),
            currency: "tokens".into(),
            limit: 150.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();

    // The override as the callers actually spell it: an internal-id AgentId.
    let internal = svc
        .list_budgets_for_session_hierarchy(1, &session.id.to_string(), Some(&agent.id.to_string()))
        .await;
    assert_eq!(
        internal.iter().filter(|b| b.id == budget.id).count(),
        1,
        "override spelled as the internal AgentId must resolve to the public_id subject"
    );

    // And an override that is already a public id must keep working.
    let public = svc
        .list_budgets_for_session_hierarchy(1, &session.id.to_string(), Some(&agent.public_id))
        .await;
    assert_eq!(
        public.iter().filter(|b| b.id == budget.id).count(),
        1,
        "override spelled as the public id must keep binding"
    );
}
