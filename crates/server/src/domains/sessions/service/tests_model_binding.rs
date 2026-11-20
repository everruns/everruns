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
        model_role: None,
        model_preset: None,
        owner_agent_id: None,
        completion_check: Default::default(),
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

#[tokio::test]
async fn research_agent_follows_the_org_research_model_until_it_pins_one() {
    let db = Arc::new(StorageBackend::test_database());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;
    let service = SessionService::new(db.clone());
    let general = create_model(&db, caller.org_id, "general").await;
    let research = create_model(&db, caller.org_id, "research").await;
    let next_research = create_model(&db, caller.org_id, "next-research").await;
    let set_research = |id| crate::storage::UpdateOrganizationSettings {
        default_model_id: UpdateField::Set(general),
        research_model_id: UpdateField::Set(id),
        ..Default::default()
    };
    db.patch_organization_settings(caller.org_id, set_research(research))
        .await
        .unwrap();

    // A role and a pinned model together are refused.
    let both = crate::domains::agents::CreateAgent(CreateAgentRequest {
        model_role: Some(crate::domains::organizations::research_model::AgentModelRole::Research),
        model_preset: None,
        default_model_id: Some(general),
        ..serde_json::from_value(serde_json::json!({"name": "both", "system_prompt": "x"})).unwrap()
    })
    .execute(&ctx)
    .await
    .unwrap_err();
    assert!(both.to_string().contains("not both"), "{both}");

    let agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
        model_role: Some(crate::domains::organizations::research_model::AgentModelRole::Research),
        model_preset: None,
        ..serde_json::from_value(serde_json::json!({"name": "researcher", "system_prompt": "x"}))
            .unwrap()
    })
    .execute(&ctx)
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(&agent).unwrap()["model_role"],
        "research"
    );
    let harness = org_init::base_harness_id(&db, caller.org_id).await.unwrap();
    let session = service
        .create(
            &caller,
            harness.uuid(),
            Some(agent.internal_id),
            Some(agent.public_id),
            SessionSource::Api,
            build_create_request(harness, Some(agent.public_id), None),
        )
        .await
        .unwrap();
    let resolved = || service.resolved_model_id(caller.org_id, &session);
    assert_eq!(resolved().await.unwrap(), Some(research));

    // Changing the org setting moves the agent without editing it.
    db.patch_organization_settings(caller.org_id, set_research(next_research))
        .await
        .unwrap();
    assert_eq!(resolved().await.unwrap(), Some(next_research));

    // Pinning a model replaces the role.
    let pinned = crate::domains::agents::UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: serde_json::from_value(serde_json::json!({"default_model_id": general.to_string()}))
            .unwrap(),
    }
    .execute(&ctx)
    .await
    .unwrap();
    assert_eq!(pinned.model_role, None);
    assert_eq!(pinned.default_model_id, Some(general));
}

#[tokio::test]
async fn agent_on_a_model_preset_runs_on_its_jobs_and_keeps_the_preset_alive() {
    use crate::domains::model_presets::{
        CreateModelPreset, CreateModelPresetRequest, DeleteModelPreset, ModelPresetJobs,
        UpdateModelPreset,
    };
    let db = Arc::new(StorageBackend::test_database());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;
    let service = SessionService::new(db.clone());
    let general = create_model(&db, caller.org_id, "general").await;
    let cheap = create_model(&db, caller.org_id, "cheap").await;
    let deep = create_model(&db, caller.org_id, "deep").await;
    let coder = create_model(&db, caller.org_id, "coder").await;
    db.patch_organization_settings(
        caller.org_id,
        crate::storage::UpdateOrganizationSettings {
            default_model_id: UpdateField::Set(general),
            coding_model_id: UpdateField::Set(coder),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let preset = CreateModelPreset(CreateModelPresetRequest {
        name: "Cheap council".into(),
        jobs: ModelPresetJobs {
            default_model_id: Some(cheap),
            research_model_id: Some(deep),
            ..Default::default()
        },
        ..Default::default()
    })
    .execute(&ctx)
    .await
    .unwrap();
    assert_eq!(preset.id, "cheap-council");
    assert!(!preset.read_only);

    // Unknown presets are refused on agents.
    let unknown = crate::domains::agents::CreateAgent(
        serde_json::from_value(serde_json::json!({
            "name": "lost", "system_prompt": "x", "model_preset": "nope"
        }))
        .unwrap(),
    )
    .execute(&ctx)
    .await
    .unwrap_err();
    assert!(unknown.to_string().contains("not found"), "{unknown}");

    let agent = crate::domains::agents::CreateAgent(
        serde_json::from_value(serde_json::json!({
            "name": "council", "system_prompt": "x", "model_preset": "cheap-council"
        }))
        .unwrap(),
    )
    .execute(&ctx)
    .await
    .unwrap();
    assert_eq!(agent.model_preset.as_deref(), Some("cheap-council"));

    // Default job on an ordinary harness.
    let turn = |agent: &crate::domains::agents::record::Agent, harness: Option<&'static str>| {
        let db = db.clone();
        let agent = agent.clone();
        async move {
            crate::domains::organizations::research_model::turn_model_id(
                &db,
                DEFAULT_ORG_ID,
                None,
                Some(&agent),
                harness,
                None,
            )
            .await
            .unwrap()
        }
    };
    assert_eq!(turn(&agent, Some("generic")).await, Some(cheap));
    // The preset leaves Coding empty: the coding harness uses the org Coding default.
    assert_eq!(turn(&agent, Some("coding")).await, Some(coder));
    // A research agent takes the preset's Research job.
    let mut researcher = agent.clone();
    researcher.model_role =
        Some(crate::domains::organizations::research_model::AgentModelRole::Research);
    assert_eq!(turn(&researcher, None).await, Some(deep));
    // A pinned model still wins.
    let mut pinned = agent.clone();
    pinned.default_model_id = Some(general);
    assert_eq!(turn(&pinned, None).await, Some(general));

    // The session shows the model it runs on.
    let harness = org_init::base_harness_id(&db, caller.org_id).await.unwrap();
    let session = service
        .create(
            &caller,
            harness.uuid(),
            Some(agent.internal_id),
            Some(agent.public_id),
            SessionSource::Api,
            build_create_request(harness, Some(agent.public_id), None),
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .resolved_model_id(caller.org_id, &session)
            .await
            .unwrap(),
        Some(cheap)
    );

    // Clearing a job falls back to the organization default.
    UpdateModelPreset {
        id: "cheap-council".into(),
        req: serde_json::from_value(serde_json::json!({"default_model_id": null})).unwrap(),
    }
    .execute(&ctx)
    .await
    .unwrap();
    assert_eq!(turn(&agent, Some("generic")).await, None);

    // A preset in use cannot be deleted; built-ins never.
    let refused = DeleteModelPreset {
        id: "cheap-council".into(),
    }
    .execute(&ctx)
    .await
    .unwrap_err();
    assert!(refused.to_string().contains("agent"), "{refused}");
    let builtin = DeleteModelPreset {
        id: "automatic".into(),
    }
    .execute(&ctx)
    .await
    .unwrap_err();
    assert!(builtin.to_string().contains("read only"), "{builtin}");

    // Moving the agent back to the organization defaults frees the preset.
    crate::domains::agents::UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: serde_json::from_value(serde_json::json!({"model_preset": null})).unwrap(),
    }
    .execute(&ctx)
    .await
    .unwrap();
    assert!(
        DeleteModelPreset {
            id: "cheap-council".into(),
        }
        .execute(&ctx)
        .await
        .unwrap()
    );
}
