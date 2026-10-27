//! Tests: a session binds a model only when one is asked for.

use super::*;
use crate::domains::agents::types::CreateAgentRequest;
use crate::domains::common::Command;
use crate::kernel_imports::{Caller, DEFAULT_ORG_ID};
use crate::storage::{StorageBackend, UpdateAgent};

use super::tests_support::*;

async fn agent_with_model(
    ctx: &crate::domains::common::Ctx,
    default_model_id: Option<ModelId>,
) -> <crate::domains::agents::CreateAgent as Command>::Output {
    crate::domains::agents::CreateAgent(CreateAgentRequest {
        localizations: Default::default(),
        service_virtual_user_id: None,
        id: None,
        name: "model-binding-agent".to_string(),
        display_name: None,
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "agent".to_string(),
        default_model_id,
        harness_id: None,
        harness_name: None,
        tags: vec![],
        capabilities: vec![],
        sandbox_policy: None,
        initial_files: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        max_output_tokens: None,
        communication: Default::default(),
    })
    .execute(ctx)
    .await
    .unwrap()
}

fn model_update(model_id: UpdateField<ModelId>) -> UpdateSessionRequest {
    UpdateSessionRequest {
        title: None,
        goal: None,
        virtual_user_id: UpdateField::Unchanged,
        model_id,
        locale: None,
        tags: None,
    }
}

#[tokio::test]
async fn session_follows_the_agent_default_model_until_one_is_chosen() {
    let db = Arc::new(StorageBackend::test_database());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;
    let service = SessionService::new(db.clone());
    let harness_id = org_init::base_harness_id(&db, caller.org_id).await.unwrap();
    let first = create_model(&db, caller.org_id, "first").await;
    let second = create_model(&db, caller.org_id, "second").await;
    let agent = agent_with_model(&ctx, Some(first)).await;

    let session = service
        .create(
            &caller,
            harness_id.uuid(),
            Some(agent.internal_id),
            Some(agent.public_id),
            SessionSource::Api,
            build_create_request(harness_id, Some(agent.public_id), None),
        )
        .await
        .unwrap();
    // The agent default is not copied onto the session.
    assert_eq!(session.model_id, None);
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &session)
            .await
            .unwrap(),
        Some(first)
    );

    // Changing the agent's default reaches the existing session's next turn.
    db.update_agent(
        caller.org_id,
        AgentId::from_uuid(agent.internal_id),
        UpdateAgent {
            default_model_id: Some(second),
            ..UpdateAgent::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &session)
            .await
            .unwrap(),
        Some(second)
    );

    // An explicit model still binds.
    let explicit = service
        .create(
            &caller,
            harness_id.uuid(),
            Some(agent.internal_id),
            Some(agent.public_id),
            SessionSource::Api,
            build_create_request(harness_id, Some(agent.public_id), Some(first)),
        )
        .await
        .unwrap();
    assert_eq!(explicit.model_id, Some(first));
}

#[tokio::test]
async fn update_sets_clears_and_validates_the_session_model() {
    let db = Arc::new(StorageBackend::test_database());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;
    let service = SessionService::new(db.clone());
    let harness_id = org_init::base_harness_id(&db, caller.org_id).await.unwrap();
    let agent_default = create_model(&db, caller.org_id, "agent-default").await;
    let chosen = create_model(&db, caller.org_id, "chosen").await;
    let agent = agent_with_model(&ctx, Some(agent_default)).await;
    let session = service
        .create(
            &caller,
            harness_id.uuid(),
            Some(agent.internal_id),
            Some(agent.public_id),
            SessionSource::Api,
            build_create_request(harness_id, Some(agent.public_id), None),
        )
        .await
        .unwrap();

    let updated = service
        .update(
            &caller,
            session.id.uuid(),
            model_update(UpdateField::Set(chosen)),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.model_id, Some(chosen));
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &updated)
            .await
            .unwrap(),
        Some(chosen)
    );

    // Unrelated updates leave the model alone.
    let retitled = service
        .update(
            &caller,
            session.id.uuid(),
            UpdateSessionRequest {
                title: Some("Renamed".to_string()),
                ..model_update(UpdateField::Unchanged)
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retitled.model_id, Some(chosen));

    let unknown = service
        .update(
            &caller,
            session.id.uuid(),
            model_update(UpdateField::Set(ModelId::new())),
        )
        .await;
    assert!(unknown.is_err(), "unknown model must be rejected");

    let cleared = service
        .update(&caller, session.id.uuid(), model_update(UpdateField::Clear))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cleared.model_id, None);
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &cleared)
            .await
            .unwrap(),
        Some(agent_default)
    );
}

#[test]
fn update_request_reads_null_as_clear_and_absent_as_unchanged() {
    let model_id = ModelId::new();
    let set: UpdateSessionRequest =
        serde_json::from_value(serde_json::json!({ "model_id": model_id.to_string() })).unwrap();
    assert!(matches!(set.model_id, UpdateField::Set(id) if id == model_id));
    let cleared: UpdateSessionRequest =
        serde_json::from_value(serde_json::json!({ "model_id": null })).unwrap();
    assert!(matches!(cleared.model_id, UpdateField::Clear));
    let absent: UpdateSessionRequest = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(matches!(absent.model_id, UpdateField::Unchanged));
}

#[tokio::test]
async fn operator_chat_uses_its_own_org_default_without_rebinding_sessions() {
    let db = Arc::new(StorageBackend::test_database());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;
    let service = SessionService::new(db.clone());
    let general = create_model(&db, caller.org_id, "agents-default").await;
    let chat_model = create_model(&db, caller.org_id, "chat-default").await;
    let next_chat = create_model(&db, caller.org_id, "next-chat").await;
    db.patch_organization_settings(
        caller.org_id,
        crate::storage::UpdateOrganizationSettings {
            default_model_id: UpdateField::Set(general),
            chat_model_id: UpdateField::Set(chat_model),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    crate::platform_chat_agent::initialize(&db, caller.org_id)
        .await
        .unwrap();
    let agent = db
        .get_agent_by_name(caller.org_id, crate::platform_chat_agent::NAME)
        .await
        .unwrap()
        .unwrap();
    let public_id = agent.public_id.parse::<AgentId>().unwrap();
    let mut session = service
        .create(
            &caller,
            agent.harness_id.uuid(),
            Some(agent.id.uuid()),
            Some(public_id),
            SessionSource::Api,
            build_create_request(agent.harness_id, Some(public_id), None),
        )
        .await
        .unwrap();
    assert_eq!(session.model_id, None);
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &session)
            .await
            .unwrap(),
        Some(chat_model)
    );
    db.patch_organization_settings(
        caller.org_id,
        crate::storage::UpdateOrganizationSettings {
            chat_model_id: UpdateField::Set(next_chat),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &session)
            .await
            .unwrap(),
        Some(next_chat)
    );
    assert_eq!(
        db.get_organization_settings(caller.org_id)
            .await
            .unwrap()
            .unwrap()
            .default_model_id,
        Some(general)
    );
    session.model_id = Some(general);
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &session)
            .await
            .unwrap(),
        Some(general)
    );
    assert_eq!(
        crate::platform_chat_agent::org_chat_model_id(
            &db,
            caller.org_id,
            crate::platform_chat_agent::NAME,
            false
        )
        .await
        .unwrap(),
        None
    );

    let ordinary = agent_with_model(&ctx, None).await;
    let harness = org_init::base_harness_id(&db, caller.org_id).await.unwrap();
    // A Work Chat / Chat-sourced conversation for a user Agent is still an Agent session.
    let ordinary_session = service
        .create(
            &caller,
            harness.uuid(),
            Some(ordinary.internal_id),
            Some(ordinary.public_id),
            SessionSource::Chat,
            build_create_request(harness, Some(ordinary.public_id), None),
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &ordinary_session)
            .await
            .unwrap(),
        Some(general)
    );
    db.update_agent(
        caller.org_id,
        public_id,
        UpdateAgent {
            default_model_id: Some(chat_model),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    session.model_id = None;
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &session)
            .await
            .unwrap(),
        Some(chat_model)
    );
}
