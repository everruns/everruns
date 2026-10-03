use super::*;
use crate::domains::agent_endpoints::types::{
    CreateAgentEndpointRequest, UpdateAgentEndpointRequest,
};
use crate::storage::StorageBackend;
use crate::storage::models::{CreateAgentRow, CreateHarnessRow};
use everruns_contracts::typed_id::AgentId;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use everruns_platform::{EndpointStatus, EndpointTransport};
use serde_json::json;
use std::sync::Arc;

async fn seed_agent(db: &StorageBackend) -> String {
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: "endpoint-harness".to_string(),
                display_name: Some("Endpoint Harness".to_string()),
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: Some(String::new()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                embedder_metadata: json!({}),
                is_built_in: false,
            },
        )
        .await
        .expect("create harness");
    let public_id = AgentId::new().to_string();
    db.create_agent(
        DEFAULT_ORG_ID,
        CreateAgentRow {
            public_id: public_id.clone(),
            name: "endpoint-agent".to_string(),
            display_name: Some("Endpoint Agent".to_string()),
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: json!([]),
            system_prompt: String::new(),
            default_model_id: None,
            harness_id: harness.id,
            tags: vec![],
            initial_files: json!([]),
            tools: json!([]),
            mcp_servers: json!({}),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            environments: None,
            is_built_in: false,
        },
    )
    .await
    .expect("create agent");
    public_id
}

fn test_ctx(db: Arc<StorageBackend>) -> Ctx {
    Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, None)
}

#[tokio::test]
async fn endpoint_commands_cover_the_management_lifecycle() {
    let db = Arc::new(StorageBackend::in_memory());
    let agent_id = seed_agent(&db).await;
    let ctx = test_ctx(db);

    let created = CreateAgentEndpoint {
        agent_id: agent_id.clone(),
        req: CreateAgentEndpointRequest {
            channel_type: EndpointTransport::Webhook,
            channel_config: json!({
                "token": "endpoint-secret",
                "message": "Process {{payload}}",
            }),
            enabled: true,
            agent_version_policy: None,
            agent_version_id: None,
        },
    }
    .run(&ctx)
    .await
    .expect("create endpoint");
    assert_eq!(created.status, EndpointStatus::Draft);
    assert!(created.enabled);

    let endpoint_id = created.public_id.to_string();
    let listed = ListAgentEndpoints {
        agent_id: agent_id.clone(),
    }
    .run(&ctx)
    .await
    .expect("list endpoints");
    assert_eq!(listed.len(), 1);

    let disabled = UpdateAgentEndpointCmd {
        agent_id: agent_id.clone(),
        endpoint_id: endpoint_id.clone(),
        req: UpdateAgentEndpointRequest {
            channel_config: None,
            enabled: Some(false),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .expect("disable endpoint");
    assert_eq!(disabled.status, EndpointStatus::Disabled);
    assert!(!disabled.enabled);

    let enabled = UpdateAgentEndpointCmd {
        agent_id: agent_id.clone(),
        endpoint_id: endpoint_id.clone(),
        req: UpdateAgentEndpointRequest {
            channel_config: None,
            enabled: Some(true),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .expect("enable endpoint");
    assert_eq!(enabled.status, EndpointStatus::Draft);

    let published = PublishAgentEndpoint {
        agent_id: agent_id.clone(),
        endpoint_id: endpoint_id.clone(),
    }
    .run(&ctx)
    .await
    .expect("publish endpoint");
    assert_eq!(published.status, EndpointStatus::Live);

    let unpublished = UnpublishAgentEndpoint {
        agent_id: agent_id.clone(),
        endpoint_id: endpoint_id.clone(),
    }
    .run(&ctx)
    .await
    .expect("unpublish endpoint");
    assert_eq!(unpublished.status, EndpointStatus::Draft);

    DeleteAgentEndpoint {
        agent_id: agent_id.clone(),
        endpoint_id,
    }
    .run(&ctx)
    .await
    .expect("delete endpoint");
    let listed = ListAgentEndpoints { agent_id }
        .run(&ctx)
        .await
        .expect("list endpoints after delete");
    assert!(listed.is_empty());
}

#[tokio::test]
async fn native_schedule_creation_uses_agent_triggers_instead() {
    let db = Arc::new(StorageBackend::in_memory());
    let agent_id = seed_agent(&db).await;
    let ctx = test_ctx(db);

    let error = CreateAgentEndpoint {
        agent_id,
        req: CreateAgentEndpointRequest {
            channel_type: EndpointTransport::Schedule,
            channel_config: json!({
                "cron_expression": "0 0 * * * * *",
                "timezone": "UTC",
                "session_mode": "endpoint",
                "message": "Run",
            }),
            enabled: true,
            agent_version_policy: None,
            agent_version_id: None,
        },
    }
    .run(&ctx)
    .await
    .expect_err("schedule endpoint creation must use agent triggers");

    assert!(matches!(
        error.kind,
        CommandErrorKind::BadRequest(message)
            if message == "Create schedules through agent triggers"
    ));
}

async fn seed_version(
    db: &StorageBackend,
    agent_public_id: &str,
    is_published: bool,
) -> everruns_contracts::typed_id::AgentVersionId {
    let agent = db
        .get_agent_by_public_id(DEFAULT_ORG_ID, agent_public_id)
        .await
        .expect("load agent")
        .expect("agent exists");
    let id = everruns_contracts::typed_id::AgentVersionId::new();
    db.create_agent_version(crate::storage::models::CreateAgentVersionRow {
        id,
        public_id: id.to_string(),
        org_id: DEFAULT_ORG_ID,
        agent_id: agent.id,
        version_number: 1,
        semver_major: 0,
        semver_minor: 1,
        semver_patch: 0,
        version: if is_published { "0.1.0" } else { "draft.1" }.to_string(),
        is_published,
        parent_version_id: None,
        source_version_id: None,
        created_by_principal_id: None,
        change_kind: if is_published { "minor" } else { "auto" }.to_string(),
        summary: None,
        config_hash: "hash".to_string(),
        authored_config: json!({}),
        resolved_config: json!({}),
    })
    .await
    .expect("create version");
    id
}

fn webhook_create(
    policy: Option<everruns_platform::AgentVersionPolicy>,
    version: Option<everruns_contracts::typed_id::AgentVersionId>,
) -> CreateAgentEndpointRequest {
    CreateAgentEndpointRequest {
        channel_type: EndpointTransport::Webhook,
        channel_config: json!({ "token": "endpoint-secret", "message": "Process {{payload}}" }),
        enabled: true,
        agent_version_policy: policy,
        agent_version_id: version,
    }
}

#[tokio::test]
async fn endpoint_version_pin_round_trips_and_unpins() {
    use everruns_platform::AgentVersionPolicy;
    let db = Arc::new(StorageBackend::in_memory());
    let agent_id = seed_agent(&db).await;
    let version = seed_version(&db, &agent_id, true).await;
    let ctx = test_ctx(db);

    let created = CreateAgentEndpoint {
        agent_id: agent_id.clone(),
        req: webhook_create(Some(AgentVersionPolicy::Pinned), Some(version)),
    }
    .run(&ctx)
    .await
    .expect("create pinned endpoint");
    assert_eq!(created.agent_version_policy, AgentVersionPolicy::Pinned);
    assert_eq!(created.agent_version_id, Some(version));
    let endpoint_id = created.public_id.to_string();

    // An update that does not mention the version leaves the pin alone.
    let touched = UpdateAgentEndpointCmd {
        agent_id: agent_id.clone(),
        endpoint_id: endpoint_id.clone(),
        req: UpdateAgentEndpointRequest {
            enabled: Some(true),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .expect("update endpoint");
    assert_eq!(touched.agent_version_policy, AgentVersionPolicy::Pinned);
    assert_eq!(touched.agent_version_id, Some(version));

    let unpinned = UpdateAgentEndpointCmd {
        agent_id: agent_id.clone(),
        endpoint_id: endpoint_id.clone(),
        req: UpdateAgentEndpointRequest {
            agent_version_policy: Some(AgentVersionPolicy::Default),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .expect("unpin endpoint");
    assert_eq!(unpinned.agent_version_policy, AgentVersionPolicy::Default);
    assert_eq!(unpinned.agent_version_id, None);

    let fetched = GetAgentEndpoint {
        agent_id,
        endpoint_id,
    }
    .run(&ctx)
    .await
    .expect("get endpoint");
    assert_eq!(fetched.agent_version_policy, AgentVersionPolicy::Default);
    assert_eq!(fetched.agent_version_id, None);
}

#[tokio::test]
async fn endpoint_version_pin_rejects_invalid_selections() {
    use everruns_platform::AgentVersionPolicy;
    let db = Arc::new(StorageBackend::in_memory());
    let agent_id = seed_agent(&db).await;
    let other_agent_id = seed_agent(&db).await;
    let foreign_version = seed_version(&db, &other_agent_id, true).await;
    let snapshot = seed_version(&db, &agent_id, false).await;
    let own_version = seed_version(&db, &agent_id, true).await;
    let ctx = test_ctx(db);

    let cases = [
        (
            Some(AgentVersionPolicy::Pinned),
            None,
            "requires agent_version_id",
        ),
        (
            Some(AgentVersionPolicy::Pinned),
            Some(foreign_version),
            "does not name a version of this agent",
        ),
        (
            Some(AgentVersionPolicy::Pinned),
            Some(everruns_contracts::typed_id::AgentVersionId::new()),
            "does not name a version of this agent",
        ),
        (
            Some(AgentVersionPolicy::Pinned),
            Some(snapshot),
            "automatic draft snapshots",
        ),
        (
            Some(AgentVersionPolicy::Latest),
            Some(own_version),
            "only valid with agent_version_policy 'pinned'",
        ),
    ];
    for (policy, version, expected) in cases {
        let error = CreateAgentEndpoint {
            agent_id: agent_id.clone(),
            req: webhook_create(policy, version),
        }
        .run(&ctx)
        .await
        .expect_err("invalid selection must be rejected");
        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(
            error.message().contains(expected),
            "expected {expected:?} in {}",
            error.message()
        );
    }
}

#[tokio::test]
async fn endpoint_version_pin_requires_agent_versions_feature() {
    use everruns_platform::AgentVersionPolicy;
    let db = Arc::new(StorageBackend::in_memory());
    let agent_id = seed_agent(&db).await;
    let version = seed_version(&db, &agent_id, true).await;
    let mut flags = crate::domains::common::all_feature_flags_for_test();
    flags.agent_versions = false;
    let ctx = test_ctx(db).with_feature_flags(flags);

    let error = CreateAgentEndpoint {
        agent_id: agent_id.clone(),
        req: webhook_create(Some(AgentVersionPolicy::Pinned), Some(version)),
    }
    .run(&ctx)
    .await
    .expect_err("pinning needs the feature");
    assert_eq!(error.code.as_deref(), Some("feature_not_enabled"));

    // Unpinning stays available so an org is never stuck in a state the flag hides.
    CreateAgentEndpoint {
        agent_id,
        req: webhook_create(Some(AgentVersionPolicy::Default), None),
    }
    .run(&ctx)
    .await
    .expect("default policy is always accepted");
}

/// Contexts for several roles over one database and one encryption key, so
/// each role reads the secrets the others stored.
fn role_ctxs(db: Arc<StorageBackend>) -> impl Fn(everruns_core::OrgRole) -> Ctx {
    let encryption = crate::storage::encryption::EncryptionService::new(
        &crate::storage::encryption::generate_encryption_key("test"),
        &[],
    )
    .expect("test encryption service");
    let base = Ctx::minimal_for_test(
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
            user_id: Some(uuid::Uuid::nil()),
            role: everruns_core::OrgRole::Owner,
            is_platform_user: false,
            is_internal: false,
        },
        db,
        Some(Arc::new(encryption)),
    );
    move |role| {
        let mut ctx = base.clone();
        ctx.caller.role = role;
        ctx
    }
}

async fn create_endpoint(
    ctx: &Ctx,
    agent_id: &str,
    channel_type: EndpointTransport,
    channel_config: serde_json::Value,
    publish: bool,
) -> String {
    let endpoint_id = CreateAgentEndpoint {
        agent_id: agent_id.to_string(),
        req: CreateAgentEndpointRequest {
            channel_type,
            channel_config,
            enabled: true,
            agent_version_policy: None,
            agent_version_id: None,
        },
    }
    .run(ctx)
    .await
    .expect("create endpoint")
    .public_id
    .to_string();
    if publish {
        PublishAgentEndpoint {
            agent_id: agent_id.to_string(),
            endpoint_id: endpoint_id.clone(),
        }
        .run(ctx)
        .await
        .expect("owner publishes endpoint");
    }
    endpoint_id
}

fn config_update(channel_config: serde_json::Value) -> UpdateAgentEndpointRequest {
    UpdateAgentEndpointRequest {
        channel_config: Some(channel_config),
        ..Default::default()
    }
}

/// Altering a live endpoint's auth, exposure, or enabled status is a
/// publication decision, so it needs the same dangerous permission as
/// publish and unpublish (EVE-1176).
#[tokio::test]
async fn live_endpoint_exposure_changes_require_dangerous_permission() {
    use everruns_core::OrgRole;
    let db = Arc::new(StorageBackend::in_memory());
    let agent_id = seed_agent(&db).await;
    let role_ctx = role_ctxs(db);
    let owner = role_ctx(OrgRole::Owner);
    let shared_secret = json!({ "token": "live-secret", "auth": { "mode": "shared_secret" } });
    let ag_ui = create_endpoint(
        &owner,
        &agent_id,
        EndpointTransport::AgUi,
        shared_secret.clone(),
        true,
    )
    .await;
    let webhook_config = json!({ "token": "endpoint-secret", "message": "Process {{payload}}" });
    let webhook = create_endpoint(
        &owner,
        &agent_id,
        EndpointTransport::Webhook,
        webhook_config.clone(),
        true,
    )
    .await;

    let attempts: Vec<(&str, &str, UpdateAgentEndpointRequest)> = vec![
        (
            "switch live auth to anonymous",
            &ag_ui,
            config_update(json!({ "auth": { "mode": "anonymous" } })),
        ),
        (
            "replace the live shared secret",
            &ag_ui,
            config_update(
                json!({ "token": "attacker-secret", "auth": { "mode": "shared_secret" } }),
            ),
        ),
        (
            "replace the live webhook token",
            &webhook,
            config_update(json!({ "token": "attacker-token", "message": "Process {{payload}}" })),
        ),
        (
            "disable a live endpoint",
            &webhook,
            UpdateAgentEndpointRequest {
                enabled: Some(false),
                ..Default::default()
            },
        ),
    ];
    for role in [OrgRole::Member, OrgRole::Admin] {
        let ctx = role_ctx(role);
        for (case, endpoint_id, req) in &attempts {
            let error = UpdateAgentEndpointCmd {
                agent_id: agent_id.clone(),
                endpoint_id: endpoint_id.to_string(),
                req: req.clone(),
            }
            .run(&ctx)
            .await
            .expect_err(case);
            assert_eq!(
                error.status(),
                axum::http::StatusCode::FORBIDDEN,
                "{role:?}: {case}"
            );
        }
        let error = UnpublishAgentEndpoint {
            agent_id: agent_id.clone(),
            endpoint_id: webhook.clone(),
        }
        .run(&ctx)
        .await
        .expect_err("manage-only member must not unpublish");
        assert_eq!(error.status(), axum::http::StatusCode::FORBIDDEN);
    }

    // Nothing the member attempted reached storage.
    let stored = GetAgentEndpoint {
        agent_id: agent_id.clone(),
        endpoint_id: ag_ui.clone(),
    }
    .run(&owner)
    .await
    .expect("get endpoint");
    assert_eq!(stored.status, EndpointStatus::Live);
    assert_eq!(
        stored.auth.as_ref().map(|auth| auth.mode.clone()),
        Some(everruns_platform::EndpointAuthMode::SharedSecret)
    );

    // Re-saving a live endpoint without changing it (secrets omitted or shown
    // as the redacted `*_configured` flags) is not an exposure change.
    let member = role_ctx(OrgRole::Member);
    let resaved = UpdateAgentEndpointCmd {
        agent_id: agent_id.clone(),
        endpoint_id: webhook.clone(),
        req: UpdateAgentEndpointRequest {
            channel_config: Some(
                json!({ "token_configured": true, "message": "Process {{payload}}" }),
            ),
            enabled: Some(true),
            ..Default::default()
        },
    }
    .run(&member)
    .await
    .expect("member may re-save a live endpoint unchanged");
    assert_eq!(resaved.status, EndpointStatus::Live);
    UpdateAgentEndpointCmd {
        agent_id: agent_id.clone(),
        endpoint_id: ag_ui.clone(),
        req: config_update(
            json!({ "token_configured": true, "auth": { "mode": "shared_secret" } }),
        ),
    }
    .run(&member)
    .await
    .expect("member may re-save live auth unchanged");

    // The owner holds the dangerous permission and may do all of it.
    for (case, endpoint_id, req) in attempts {
        UpdateAgentEndpointCmd {
            agent_id: agent_id.clone(),
            endpoint_id: endpoint_id.to_string(),
            req,
        }
        .run(&owner)
        .await
        .unwrap_or_else(|error| panic!("owner may {case}: {error:?}"));
    }
    UnpublishAgentEndpoint {
        agent_id: agent_id.clone(),
        endpoint_id: ag_ui,
    }
    .run(&owner)
    .await
    .expect("owner may unpublish");
}

#[tokio::test]
async fn draft_endpoint_edits_stay_available_to_managers() {
    use everruns_core::OrgRole;
    let db = Arc::new(StorageBackend::in_memory());
    let agent_id = seed_agent(&db).await;
    let member = role_ctxs(db)(OrgRole::Member);
    let endpoint_id = create_endpoint(
        &member,
        &agent_id,
        EndpointTransport::AgUi,
        json!({ "token": "draft-secret", "auth": { "mode": "shared_secret" } }),
        false,
    )
    .await;

    for req in [
        config_update(json!({ "auth": { "mode": "anonymous" } })),
        config_update(json!({ "token": "rotated", "auth": { "mode": "shared_secret" } })),
        UpdateAgentEndpointRequest {
            enabled: Some(false),
            ..Default::default()
        },
        UpdateAgentEndpointRequest {
            enabled: Some(true),
            ..Default::default()
        },
    ] {
        let updated = UpdateAgentEndpointCmd {
            agent_id: agent_id.clone(),
            endpoint_id: endpoint_id.clone(),
            req,
        }
        .run(&member)
        .await
        .expect("member may edit a draft endpoint");
        assert_ne!(updated.status, EndpointStatus::Live);
    }
}

#[tokio::test]
async fn agent_channel_summaries_are_page_scoped_and_exclude_triggers() {
    let db = StorageBackend::in_memory();
    let public_id = seed_agent(&db).await;
    let agent = db
        .get_agent_by_public_id(DEFAULT_ORG_ID, &public_id)
        .await
        .unwrap()
        .unwrap();
    for (kind, enabled, status) in [
        ("webhook", true, "live"),
        ("api_endpoint", false, "disabled"),
        ("schedule", true, "live"),
    ] {
        db.create_agent_endpoint(
            DEFAULT_ORG_ID,
            crate::storage::CreateAgentEndpointRow {
                agent_id: agent.id.uuid(),
                public_id: format!("aep_{kind}"),
                channel_type: kind.into(),
                channel_config: json!({"token": "must-not-be-projected"}),
                channel_config_encrypted: None,
                auth: None,
                auth_encrypted: None,
                enabled,
                status: status.into(),
                virtual_user_id: None,
                agent_version_policy: "default".into(),
                agent_version_id: None,
                owner_principal_id: uuid::Uuid::now_v7(),
                resolved_owner_user_id: None,
            },
        )
        .await
        .unwrap();
    }
    let rows = db
        .list_agent_channel_summaries(DEFAULT_ORG_ID, &[agent.id.uuid()])
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].public_id, "aep_webhook");
    assert!(rows[0].enabled);
    assert_eq!(rows[0].status, "live");
    assert_eq!(rows[1].channel_type, "api_endpoint");
    assert!(!rows[1].enabled);
    assert_eq!(rows[1].status, "disabled");
    assert!(
        db.list_agent_channel_summaries(DEFAULT_ORG_ID + 1, &[agent.id.uuid()])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        db.list_agent_channel_summaries(DEFAULT_ORG_ID, &[uuid::Uuid::now_v7()])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        db.list_agent_channel_summaries(DEFAULT_ORG_ID, &[])
            .await
            .unwrap()
            .is_empty()
    );
}
