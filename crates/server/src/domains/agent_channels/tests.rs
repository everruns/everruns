use super::*;
use crate::domains::agent_channels::types::{CreateAgentChannelRequest, UpdateAgentChannelRequest};
use crate::records::{ChannelStatus, ChannelType};
use crate::storage::StorageBackend;
use crate::storage::models::{CreateAgentRow, CreateHarnessRow};
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::json;
use std::sync::Arc;

async fn seed_agent(db: &StorageBackend) -> String {
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: format!("channel-harness-{}", uuid::Uuid::now_v7().simple()),
                display_name: Some("Channel Harness".to_string()),
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
            name: format!("channel-agent-{}", uuid::Uuid::now_v7().simple()),
            display_name: Some("Channel Agent".to_string()),
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
async fn create_slack_channel_cannot_forge_managed_app_removal_credentials() {
    let db = Arc::new(StorageBackend::test_database());
    let agent_id = seed_agent(&db).await;
    let ctx = test_ctx(db);
    let channel = CreateAgentChannel { agent_id: agent_id.clone(), req: CreateAgentChannelRequest {
        channel_type: ChannelType::Slack,
        channel_config: json!({"bot_token":"manual-token", "provisioned_app":{"app_id":"A-victim","client_id":"client","client_secret":"secret"}}),
        enabled: true, agent_version_policy: None, agent_version_id: None,
    }}.run(&ctx).await.unwrap();
    assert!(
        channel
            .channel_config
            .get("slack_app_provisioned")
            .is_none()
    );
    let agent = ctx
        .db
        .get_agent_by_public_id(ctx.org_id(), &agent_id)
        .await
        .unwrap()
        .unwrap();
    let row = ctx
        .db
        .get_agent_channel(
            ctx.org_id(),
            agent.id.uuid(),
            &channel.public_id.to_string(),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(row.channel_config.get("provisioned_app").is_none());
}

#[tokio::test]
async fn endpoint_commands_cover_the_management_lifecycle() {
    let db = Arc::new(StorageBackend::test_database());
    let agent_id = seed_agent(&db).await;
    let ctx = test_ctx(db);

    let created = CreateAgentChannel {
        agent_id: agent_id.clone(),
        req: CreateAgentChannelRequest {
            channel_type: ChannelType::Webhook,
            channel_config: json!({
                "token": "channel-secret",
                "message": "Process {{payload}}",
            }),
            enabled: true,
            agent_version_policy: None,
            agent_version_id: None,
        },
    }
    .run(&ctx)
    .await
    .expect("create channel");
    assert_eq!(created.status, ChannelStatus::Draft);
    assert!(created.enabled);

    let channel_id = created.public_id.to_string();
    let listed = ListAgentChannels {
        agent_id: agent_id.clone(),
    }
    .run(&ctx)
    .await
    .expect("list channels");
    assert_eq!(listed.len(), 1);

    let disabled = UpdateAgentChannelCmd {
        agent_id: agent_id.clone(),
        channel_id: channel_id.clone(),
        req: UpdateAgentChannelRequest {
            channel_config: None,
            enabled: Some(false),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .expect("disable channel");
    assert_eq!(disabled.status, ChannelStatus::Disabled);
    assert!(!disabled.enabled);

    let enabled = UpdateAgentChannelCmd {
        agent_id: agent_id.clone(),
        channel_id: channel_id.clone(),
        req: UpdateAgentChannelRequest {
            channel_config: None,
            enabled: Some(true),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .expect("enable channel");
    assert_eq!(enabled.status, ChannelStatus::Draft);

    let published = PublishAgentChannel {
        agent_id: agent_id.clone(),
        channel_id: channel_id.clone(),
    }
    .run(&ctx)
    .await
    .expect("publish channel");
    assert_eq!(published.status, ChannelStatus::Live);

    let unpublished = UnpublishAgentChannel {
        agent_id: agent_id.clone(),
        channel_id: channel_id.clone(),
    }
    .run(&ctx)
    .await
    .expect("unpublish channel");
    assert_eq!(unpublished.status, ChannelStatus::Draft);

    DeleteAgentChannel {
        agent_id: agent_id.clone(),
        channel_id,
    }
    .run(&ctx)
    .await
    .expect("delete channel");
    let listed = ListAgentChannels { agent_id }
        .run(&ctx)
        .await
        .expect("list channels after delete");
    assert!(listed.is_empty());
}

#[tokio::test]
async fn native_schedule_creation_uses_agent_triggers_instead() {
    let db = Arc::new(StorageBackend::test_database());
    let agent_id = seed_agent(&db).await;
    let ctx = test_ctx(db);

    let error = CreateAgentChannel {
        agent_id,
        req: CreateAgentChannelRequest {
            channel_type: ChannelType::Schedule,
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
    .expect_err("schedule channel creation must use agent triggers");

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
    let version_number: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version_number), 0) + 1 FROM agent_versions WHERE agent_id = $1",
    )
    .bind(agent.id.uuid())
    .fetch_one(db.database().pool())
    .await
    .expect("next version number");
    db.create_agent_version(crate::storage::models::CreateAgentVersionRow {
        id,
        public_id: id.to_string(),
        org_id: DEFAULT_ORG_ID,
        agent_id: agent.id,
        version_number,
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
    policy: Option<crate::records::AgentVersionPolicy>,
    version: Option<everruns_contracts::typed_id::AgentVersionId>,
) -> CreateAgentChannelRequest {
    CreateAgentChannelRequest {
        channel_type: ChannelType::Webhook,
        channel_config: json!({ "token": "channel-secret", "message": "Process {{payload}}" }),
        enabled: true,
        agent_version_policy: policy,
        agent_version_id: version,
    }
}

#[tokio::test]
async fn endpoint_version_pin_round_trips_and_unpins() {
    use crate::records::AgentVersionPolicy;
    let db = Arc::new(StorageBackend::test_database());
    let agent_id = seed_agent(&db).await;
    let version = seed_version(&db, &agent_id, true).await;
    let ctx = test_ctx(db);

    let created = CreateAgentChannel {
        agent_id: agent_id.clone(),
        req: webhook_create(Some(AgentVersionPolicy::Pinned), Some(version)),
    }
    .run(&ctx)
    .await
    .expect("create pinned channel");
    assert_eq!(created.agent_version_policy, AgentVersionPolicy::Pinned);
    assert_eq!(created.agent_version_id, Some(version));
    let channel_id = created.public_id.to_string();

    // An update that does not mention the version leaves the pin alone.
    let touched = UpdateAgentChannelCmd {
        agent_id: agent_id.clone(),
        channel_id: channel_id.clone(),
        req: UpdateAgentChannelRequest {
            enabled: Some(true),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .expect("update channel");
    assert_eq!(touched.agent_version_policy, AgentVersionPolicy::Pinned);
    assert_eq!(touched.agent_version_id, Some(version));

    let unpinned = UpdateAgentChannelCmd {
        agent_id: agent_id.clone(),
        channel_id: channel_id.clone(),
        req: UpdateAgentChannelRequest {
            agent_version_policy: Some(AgentVersionPolicy::Default),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .expect("unpin channel");
    assert_eq!(unpinned.agent_version_policy, AgentVersionPolicy::Default);
    assert_eq!(unpinned.agent_version_id, None);

    let fetched = GetAgentChannel {
        agent_id,
        channel_id,
    }
    .run(&ctx)
    .await
    .expect("get channel");
    assert_eq!(fetched.agent_version_policy, AgentVersionPolicy::Default);
    assert_eq!(fetched.agent_version_id, None);
}

#[tokio::test]
async fn endpoint_version_pin_rejects_invalid_selections() {
    use crate::records::AgentVersionPolicy;
    let db = Arc::new(StorageBackend::test_database());
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
        let error = CreateAgentChannel {
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
    use crate::records::AgentVersionPolicy;
    let db = Arc::new(StorageBackend::test_database());
    let agent_id = seed_agent(&db).await;
    let version = seed_version(&db, &agent_id, true).await;
    let mut flags = crate::domains::common::all_feature_flags_for_test();
    flags.agent_versions = false;
    let ctx = test_ctx(db).with_feature_flags(flags);

    let error = CreateAgentChannel {
        agent_id: agent_id.clone(),
        req: webhook_create(Some(AgentVersionPolicy::Pinned), Some(version)),
    }
    .run(&ctx)
    .await
    .expect_err("pinning needs the feature");
    assert_eq!(error.code.as_deref(), Some("feature_not_enabled"));

    // Unpinning stays available so an org is never stuck in a state the flag hides.
    CreateAgentChannel {
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

async fn create_channel(
    ctx: &Ctx,
    agent_id: &str,
    channel_type: ChannelType,
    channel_config: serde_json::Value,
    publish: bool,
) -> String {
    let channel_id = CreateAgentChannel {
        agent_id: agent_id.to_string(),
        req: CreateAgentChannelRequest {
            channel_type,
            channel_config,
            enabled: true,
            agent_version_policy: None,
            agent_version_id: None,
        },
    }
    .run(ctx)
    .await
    .expect("create channel")
    .public_id
    .to_string();
    if publish {
        PublishAgentChannel {
            agent_id: agent_id.to_string(),
            channel_id: channel_id.clone(),
        }
        .run(ctx)
        .await
        .expect("owner publishes channel");
    }
    channel_id
}

fn config_update(channel_config: serde_json::Value) -> UpdateAgentChannelRequest {
    UpdateAgentChannelRequest {
        channel_config: Some(channel_config),
        ..Default::default()
    }
}

/// Altering a live channel's auth, exposure, or enabled status is a
/// publication decision, so it needs the same dangerous permission as
/// publish and unpublish (EVE-1176).
#[tokio::test]
async fn live_channel_exposure_changes_require_dangerous_permission() {
    use everruns_core::OrgRole;
    let db = Arc::new(StorageBackend::test_database());
    let agent_id = seed_agent(&db).await;
    let role_ctx = role_ctxs(db);
    let owner = role_ctx(OrgRole::Owner);
    let shared_secret = json!({ "token": "live-secret", "auth": { "mode": "shared_secret" } });
    let ag_ui = create_channel(
        &owner,
        &agent_id,
        ChannelType::AgUi,
        shared_secret.clone(),
        true,
    )
    .await;
    let webhook_config = json!({ "token": "channel-secret", "message": "Process {{payload}}" });
    let webhook = create_channel(
        &owner,
        &agent_id,
        ChannelType::Webhook,
        webhook_config.clone(),
        true,
    )
    .await;

    let attempts: Vec<(&str, &str, UpdateAgentChannelRequest)> = vec![
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
            "disable a live channel",
            &webhook,
            UpdateAgentChannelRequest {
                enabled: Some(false),
                ..Default::default()
            },
        ),
    ];
    for role in [OrgRole::Member, OrgRole::Admin] {
        let ctx = role_ctx(role);
        for (case, channel_id, req) in &attempts {
            let error = UpdateAgentChannelCmd {
                agent_id: agent_id.clone(),
                channel_id: channel_id.to_string(),
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
        let error = UnpublishAgentChannel {
            agent_id: agent_id.clone(),
            channel_id: webhook.clone(),
        }
        .run(&ctx)
        .await
        .expect_err("manage-only member must not unpublish");
        assert_eq!(error.status(), axum::http::StatusCode::FORBIDDEN);
    }

    // Nothing the member attempted reached storage.
    let stored = GetAgentChannel {
        agent_id: agent_id.clone(),
        channel_id: ag_ui.clone(),
    }
    .run(&owner)
    .await
    .expect("get channel");
    assert_eq!(stored.status, ChannelStatus::Live);
    assert_eq!(
        stored.auth.as_ref().map(|auth| auth.mode.clone()),
        Some(crate::records::ChannelAuthMode::SharedSecret)
    );

    // Re-saving a live channel without changing it (secrets omitted or shown
    // as the redacted `*_configured` flags) is not an exposure change.
    let member = role_ctx(OrgRole::Member);
    let resaved = UpdateAgentChannelCmd {
        agent_id: agent_id.clone(),
        channel_id: webhook.clone(),
        req: UpdateAgentChannelRequest {
            channel_config: Some(
                json!({ "token_configured": true, "message": "Process {{payload}}" }),
            ),
            enabled: Some(true),
            ..Default::default()
        },
    }
    .run(&member)
    .await
    .expect("member may re-save a live channel unchanged");
    assert_eq!(resaved.status, ChannelStatus::Live);
    UpdateAgentChannelCmd {
        agent_id: agent_id.clone(),
        channel_id: ag_ui.clone(),
        req: config_update(
            json!({ "token_configured": true, "auth": { "mode": "shared_secret" } }),
        ),
    }
    .run(&member)
    .await
    .expect("member may re-save live auth unchanged");

    // The owner holds the dangerous permission and may do all of it.
    for (case, channel_id, req) in attempts {
        UpdateAgentChannelCmd {
            agent_id: agent_id.clone(),
            channel_id: channel_id.to_string(),
            req,
        }
        .run(&owner)
        .await
        .unwrap_or_else(|error| panic!("owner may {case}: {error:?}"));
    }
    UnpublishAgentChannel {
        agent_id: agent_id.clone(),
        channel_id: ag_ui,
    }
    .run(&owner)
    .await
    .expect("owner may unpublish");
}

#[tokio::test]
async fn draft_channel_edits_stay_available_to_managers() {
    use everruns_core::OrgRole;
    let db = Arc::new(StorageBackend::test_database());
    let agent_id = seed_agent(&db).await;
    let member = role_ctxs(db)(OrgRole::Member);
    let channel_id = create_channel(
        &member,
        &agent_id,
        ChannelType::AgUi,
        json!({ "token": "draft-secret", "auth": { "mode": "shared_secret" } }),
        false,
    )
    .await;

    for req in [
        config_update(json!({ "auth": { "mode": "anonymous" } })),
        config_update(json!({ "token": "rotated", "auth": { "mode": "shared_secret" } })),
        UpdateAgentChannelRequest {
            enabled: Some(false),
            ..Default::default()
        },
        UpdateAgentChannelRequest {
            enabled: Some(true),
            ..Default::default()
        },
    ] {
        let updated = UpdateAgentChannelCmd {
            agent_id: agent_id.clone(),
            channel_id: channel_id.clone(),
            req,
        }
        .run(&member)
        .await
        .expect("member may edit a draft channel");
        assert_ne!(updated.status, ChannelStatus::Live);
    }
}

#[tokio::test]
async fn agent_channel_summaries_are_page_scoped_and_exclude_triggers() {
    let db = StorageBackend::test_database();
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
        db.create_agent_channel(
            DEFAULT_ORG_ID,
            crate::storage::CreateAgentChannelRow {
                agent_id: agent.id.uuid(),
                public_id: format!("appchan_{}", uuid::Uuid::now_v7().simple()),
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
                owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1).uuid(),
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
    assert_eq!(rows[0].channel_type, "webhook");
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

#[tokio::test]
async fn slack_response_policies_are_available_without_feature_enrollment() {
    let db = Arc::new(StorageBackend::test_database());
    let agent_id = seed_agent(&db).await;
    let ctx = test_ctx(db).with_feature_flags(crate::records::FeatureFlags::default());
    let create = |policy: &str| CreateAgentChannel {
        agent_id: agent_id.clone(),
        req: CreateAgentChannelRequest {
            channel_type: ChannelType::Slack,
            channel_config: json!({"response_policy": policy}),
            enabled: true,
            agent_version_policy: None,
            agent_version_id: None,
        },
    };
    for policy in ["mentions_only", "relevant_messages"] {
        let endpoint = create(policy).run(&ctx).await.unwrap();
        assert_eq!(endpoint.channel_config["response_policy"], policy);
        let reset = UpdateAgentChannelCmd {
            agent_id: agent_id.clone(),
            channel_id: endpoint.public_id.to_string(),
            req: UpdateAgentChannelRequest {
                channel_config: Some(json!({"response_policy": "all_messages"})),
                ..Default::default()
            },
        }
        .run(&ctx)
        .await
        .unwrap();
        assert_eq!(reset.channel_config["response_policy"], "all_messages");
    }
    assert!(create("not_a_policy").run(&ctx).await.is_err());
}

#[tokio::test]
async fn native_slack_channel_reads_normalize_stored_progress_mode() {
    for encrypted in [false, true] {
        let db = Arc::new(StorageBackend::test_database());
        let agent_id = seed_agent(&db).await;
        let encryption = encrypted.then(|| {
            Arc::new(
                crate::storage::EncryptionService::new(
                    &crate::storage::encryption::generate_encryption_key("channel-mode-test"),
                    &[],
                )
                .unwrap(),
            )
        });
        let ctx = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, encryption);
        let created = CreateAgentChannel {
            agent_id: agent_id.clone(),
            req: CreateAgentChannelRequest {
                channel_type: ChannelType::Slack,
                channel_config: json!({"reply_mode":"report_progress_only", "bot_token":"xoxb-test", "signing_secret":"s"}),
                enabled: true, agent_version_policy: None, agent_version_id: None,
            },
        }.run(&ctx).await.unwrap();
        let channel_id = created.public_id.to_string();
        assert_eq!(created.channel_config["reply_mode"], "tool_only");
        let fetched = GetAgentChannel {
            agent_id: agent_id.clone(),
            channel_id,
        }
        .run(&ctx)
        .await
        .unwrap();
        let listed = ListAgentChannels { agent_id }.run(&ctx).await.unwrap();
        for endpoint in [fetched, listed.into_iter().next().unwrap()] {
            assert_eq!(endpoint.channel_config["reply_mode"], "tool_only");
            assert_eq!(endpoint.channel_config["bot_token_configured"], true);
            assert!(endpoint.channel_config.get("bot_token").is_none());
        }
    }
}
