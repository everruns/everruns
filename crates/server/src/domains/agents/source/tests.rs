use super::test_support::*;
use super::*;
use crate::storage::CreateVirtualUserConnectionRow;
use std::sync::Arc;

#[tokio::test]
async fn package_folders_with_manifest_extensions_are_read_as_directories() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, ResponseTemplate};

    let (ctx, remote, id) = setup().await;
    package_with_skills(&remote, 'a', "Nested folder instructions", None, false).await;
    let parent_path = format!("/repos/acme/agents/git/trees/{}", "b".repeat(40));
    let entries = ctx
        .github_apps
        .api
        .read_source_json(&parent_path, "source-test-token")
        .await
        .unwrap();
    get(
        &remote,
        &format!("/repos/acme/agents/git/trees/{}", "c".repeat(40)),
        entries,
    )
    .await;
    Mock::given(method("GET"))
        .and(path(parent_path))
        .and(header("authorization", "Bearer source-test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "truncated": false,
            "tree": [{"path":"support.md","mode":"040000","type":"tree","sha":"c".repeat(40)}]
        })))
        .with_priority(1)
        .mount(&remote)
        .await;
    let preview = PreviewAgentSource {
        agent_id: id,
        req: SourceLocation {
            path: "support.md".into(),
            ..location()
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(preview["valid"], true, "{preview}");
    assert_eq!(
        preview["preview"]["instructions"],
        "Nested folder instructions"
    );
}

#[test]
fn source_locations_reject_paths_and_refs_that_escape_the_selected_repository() {
    for path in [
        "../agent.toml",
        "/agent.toml",
        "a/../b",
        ".git/config",
        "a\\b",
        "a//b",
    ] {
        let mut loc = location();
        loc.path = path.into();
        assert!(loc.validate().is_err(), "{path}");
    }
    for branch in ["", "bad\nref", "main~1", "a..b"] {
        let mut loc = location();
        loc.branch = branch.into();
        assert!(loc.validate().is_err());
    }
    let mut loc = location();
    loc.branch = "feature/agents".into();
    loc.path = "agents/support".into();
    assert!(loc.validate().is_ok());
}
#[tokio::test]
async fn connects_full_package_locks_all_command_surfaces_and_disconnects_without_data_loss() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "Synced instructions", None).await;
    let source = connect(&ctx, &id).await;
    assert_eq!(source.applied_commit, Some("a".repeat(40)));
    let synced_agent = agent(&ctx, &id).await.unwrap();
    assert_eq!(synced_agent.system_prompt, "Synced instructions");
    assert!(
        synced_agent
            .initial_files
            .iter()
            .any(|f| f.path.ends_with("data/runbook.txt"))
    );
    assert!(
        synced_agent
            .initial_files
            .iter()
            .any(|f| f.path.ends_with(".agents/skills/helper/SKILL.md"))
    );
    let error = run(
        &ctx,
        "update_agent",
        json!({"id":id,"system_prompt":"Bypass"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
    let error = run(
        &ctx,
        "upsert_agent",
        json!({"id":id,"name":"source-agent","system_prompt":"Bypass"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
    let error = packages::ImportAgent::Package(packages::PackageInput {
        content: "---\nname: source-agent\n---\nBypass".into(),
        file: None,
        format: Some("markdown".into()),
        target: Some(id.clone()),
    })
    .run(&ctx)
    .await
    .unwrap_err();
    assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
    assert!(packages::export(&ctx, &id).await.is_ok());
    DisconnectAgentSource {
        agent_id: id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(
        agent(&ctx, &id).await.unwrap().system_prompt,
        "Synced instructions"
    );
    run(
        &ctx,
        "update_agent",
        json!({"id":id,"system_prompt":"Editable again"}),
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn sync_applies_new_revision_and_disconnected_inflight_work_cannot_overwrite_local_edits() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "First", None).await;
    connect(&ctx, &id).await;
    remote.reset().await;
    package(&remote, 'c', "Second", None).await;
    let row = binding(&ctx, &id).await.unwrap();
    ctx.db
        .request_agent_source_sync(ctx.org_id(), row.agent_id)
        .await
        .unwrap();
    let claim = ctx.db.claim_agent_source_sync().await.unwrap().unwrap();
    sync(&ctx, &claim).await.unwrap();
    assert_eq!(agent(&ctx, &id).await.unwrap().system_prompt, "Second");
    assert_eq!(
        binding(&ctx, &id).await.unwrap().applied_commit,
        Some("c".repeat(40))
    );
    ctx.db
        .request_agent_source_sync(ctx.org_id(), row.agent_id)
        .await
        .unwrap();
    let stale = ctx.db.claim_agent_source_sync().await.unwrap().unwrap();
    assert!(ctx.db.claim_agent_source_sync().await.unwrap().is_none());
    DisconnectAgentSource {
        agent_id: id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap();
    run(
        &ctx,
        "update_agent",
        json!({"id":id,"system_prompt":"Local"}),
    )
    .await
    .unwrap();
    sync(&ctx, &stale).await.unwrap();
    assert_eq!(agent(&ctx, &id).await.unwrap().system_prompt, "Local");
}
#[tokio::test]
async fn invalid_revision_rolls_back_agent_and_keeps_applied_commit() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "Good", None).await;
    connect(&ctx, &id).await;
    remote.reset().await;
    package(&remote, 'd', "Bad", Some("missing-capability")).await;
    let row = binding(&ctx, &id).await.unwrap();
    ctx.db
        .request_agent_source_sync(ctx.org_id(), row.agent_id)
        .await
        .unwrap();
    let claim = ctx.db.claim_agent_source_sync().await.unwrap().unwrap();
    assert!(sync(&ctx, &claim).await.is_err());
    assert_eq!(agent(&ctx, &id).await.unwrap().system_prompt, "Good");
    assert_eq!(
        binding(&ctx, &id).await.unwrap().applied_commit,
        Some("a".repeat(40))
    );
    ctx.db
        .finish_agent_source_sync(&claim, None, None, None, Some("Invalid package"))
        .await
        .unwrap();
    assert_eq!(binding(&ctx, &id).await.unwrap().status, "error");
}
#[tokio::test]
async fn changed_preview_blocks_connect_and_revoked_connection_keeps_readonly() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "First", None).await;
    let preview = PreviewAgentSource {
        agent_id: id.clone(),
        req: location(),
    }
    .run(&ctx)
    .await
    .unwrap();
    remote.reset().await;
    package(&remote, 'c', "Changed", None).await;
    let error = ConnectAgentSource {
        agent_id: id.clone(),
        req: ConnectSourceRequest {
            mode: SourceMode::Pull,
            location: location(),
            commit: preview["commit"].as_str().unwrap().into(),
            agent_digest: preview["agent_digest"].as_str().unwrap().into(),
            expected_generation: None,
        },
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
    assert!(
        GetAgentSource {
            agent_id: id.clone()
        }
        .run(&ctx)
        .await
        .unwrap()
        .is_none()
    );
    connect(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    ctx.db
        .delete_virtual_user_connection(VirtualUserId::from_uuid(row.identity_id), "github")
        .await
        .unwrap();
    ctx.db
        .request_agent_source_sync(ctx.org_id(), row.agent_id)
        .await
        .unwrap();
    let claim = ctx.db.claim_agent_source_sync().await.unwrap().unwrap();
    let error = sync(&ctx, &claim).await.unwrap_err();
    assert!(error.message().contains("disconnected"));
    assert_eq!(agent(&ctx, &id).await.unwrap().system_prompt, "Changed");
    assert!(
        run(
            &ctx,
            "update_agent",
            json!({"id":id,"system_prompt":"Bypass"})
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn applied_revision_and_definition_roll_back_together_on_storage_failure() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "Good", None).await;
    connect(&ctx, &id).await;
    remote.reset().await;
    package(&remote, 'd', "Must roll back", None).await;
    // A failure after the composed import must roll back the definition and its history too.
    sqlx::query("ALTER TABLE agent_github_sources ADD CONSTRAINT reject_test_revision CHECK (applied_commit <> 'dddddddddddddddddddddddddddddddddddddddd')")
        .execute(ctx.db.pool()).await.unwrap();
    let row = binding(&ctx, &id).await.unwrap();
    ctx.db
        .request_agent_source_sync(ctx.org_id(), row.agent_id)
        .await
        .unwrap();
    let claim = ctx.db.claim_agent_source_sync().await.unwrap().unwrap();
    assert!(sync(&ctx, &claim).await.is_err());
    assert_eq!(agent(&ctx, &id).await.unwrap().system_prompt, "Good");
    assert_eq!(
        binding(&ctx, &id).await.unwrap().applied_commit,
        Some("a".repeat(40))
    );
}

#[tokio::test]
async fn push_wakeups_match_tenant_receiving_identity_repository_and_branch() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "Good", None).await;
    connect(&ctx, &id).await;
    let original = binding(&ctx, &id).await.unwrap();
    for (org, identity, repo, branch) in [
        (
            ctx.org_id() + 1,
            original.identity_id,
            original.repository_id,
            "main",
        ),
        (ctx.org_id(), Uuid::new_v4(), original.repository_id, "main"),
        (
            ctx.org_id(),
            original.identity_id,
            original.repository_id + 1,
            "main",
        ),
        (
            ctx.org_id(),
            original.identity_id,
            original.repository_id,
            "other",
        ),
    ] {
        ctx.db
            .request_agent_source_push(org, &[identity], repo, branch)
            .await
            .unwrap();
        assert_eq!(
            binding(&ctx, &id).await.unwrap().next_sync_at,
            original.next_sync_at
        );
    }
    ctx.db
        .request_agent_source_push(
            ctx.org_id(),
            &[original.identity_id],
            original.repository_id,
            "main",
        )
        .await
        .unwrap();
    assert!(ctx.db.claim_agent_source_sync().await.unwrap().is_some());
    assert!(ctx.db.claim_agent_source_sync().await.unwrap().is_none());
    assert!(
        GetAgentSource { agent_id: id }
            .run(&Ctx {
                caller: Caller::internal(ctx.org_id() + 1),
                ..ctx
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn background_failure_is_visible_and_does_not_unlock_definition() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "Good", None).await;
    connect(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    ctx.db
        .delete_virtual_user_connection(VirtualUserId::from_uuid(row.identity_id), "github")
        .await
        .unwrap();
    ctx.db
        .request_agent_source_sync(ctx.org_id(), row.agent_id)
        .await
        .unwrap();
    assert!(reconcile_one(&ctx).await.unwrap());
    let after = binding(&ctx, &id).await.unwrap();
    assert_eq!(after.status, "error");
    assert!(after.error.unwrap().contains("disconnected"));
    assert_eq!(after.applied_commit, Some("a".repeat(40)));
    assert!(
        run(
            &ctx,
            "update_agent",
            json!({"id":id,"system_prompt":"Bypass"})
        )
        .await
        .is_err()
    );
}

#[test]
fn source_connect_request_flattens_the_reviewed_location() {
    let request: ConnectAgentSource = serde_json::from_value(json!({"agent_id":"a","repository":"acme/agents","branch":"main","path":"agent.toml","commit":"a".repeat(40),"agent_digest":"reviewed"})).unwrap();
    assert_eq!(request.req.location.repository, "acme/agents");
    assert_eq!(request.req.commit, "a".repeat(40));
}

#[tokio::test]
async fn source_owned_channels_are_readonly_but_publication_binding_remains_local() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "Good", None).await;
    let source = connect(&ctx, &id).await;
    assert_eq!(source.managed_channels, vec!["chat"]);
    let channels = crate::domains::agent_channels::ListAgentChannels {
        agent_id: id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap();
    let channel = &channels[0];
    let mut config = channel.channel_config.clone();
    config["rate_limit_per_minute"] = json!(7);
    let error = run(
        &ctx,
        "update_agent_channel",
        json!({"agent_id":id,"channel_id":channel.public_id,"channel_config":config}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
    let error = run(
        &ctx,
        "delete_agent_channel",
        json!({"agent_id":id,"channel_id":channel.public_id}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
    run(
        &ctx,
        "update_agent_channel",
        json!({"agent_id":id,"channel_id":channel.public_id,"enabled":false}),
    )
    .await
    .unwrap();
    let local = run(
        &ctx,
        "create_agent_channel",
        json!({"agent_id":id,"channel_type":"ag_ui","channel_config":{}}),
    )
    .await
    .unwrap();
    let error = run(
        &ctx,
        "update_agent_channel",
        json!({"agent_id":id,"channel_id":local["id"],"channel_config":{"package_name":"chat"}}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
    // The runtime service identity is a destination binding, not a portable authored field.
    run(
        &ctx,
        "update_agent",
        json!({"id":id,"service_virtual_user_id":null}),
    )
    .await
    .unwrap();
    assert!(
        GetAgentSource { agent_id: id }
            .run(&ctx)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn background_sync_does_not_bypass_custom_permissions_or_removed_membership() {
    struct Deny;
    impl everruns_core::PermissionResolver for Deny {
        fn has_permission(&self, _: &Caller, _: &everruns_core::Permission) -> bool {
            false
        }
        fn caller_permissions(&self, _: &Caller) -> Vec<everruns_core::Permission> {
            vec![]
        }
    }
    let (mut ctx, remote, id) = setup().await;
    package(&remote, 'a', "Good", None).await;
    connect(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    ctx.db
        .request_agent_source_sync(ctx.org_id(), row.agent_id)
        .await
        .unwrap();
    ctx.permission_resolver = Arc::new(Deny);
    assert!(reconcile_one(&ctx).await.unwrap());
    let after = binding(&ctx, &id).await.unwrap();
    assert_eq!(after.status, "error");
    assert!(after.error.unwrap().contains("permission"));

    ctx.permission_resolver = Arc::new(everruns_core::DefaultPermissionResolver);
    let mut binding_row = binding(&ctx, &id).await.unwrap();
    binding_row.configured_by = Some(Uuid::new_v4());
    ctx.db.set_agent_source(&binding_row).await.unwrap();
    ctx.db
        .request_agent_source_sync(ctx.org_id(), row.agent_id)
        .await
        .unwrap();
    assert!(reconcile_one(&ctx).await.unwrap());
    let after = binding(&ctx, &id).await.unwrap();
    assert_eq!(after.status, "error");
    assert!(
        after
            .error
            .unwrap()
            .contains("no longer an organization member")
    );
    assert_eq!(agent(&ctx, &id).await.unwrap().system_prompt, "Good");
}

#[tokio::test]
async fn replacing_installation_on_the_same_connection_requires_explicit_rebinding() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "Good", None).await;
    connect(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    let replacement = ctx
        .db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: VirtualUserId::from_uuid(row.identity_id),
            provider: "github".into(),
            connection_type: crate::github_apps::GITHUB_APP_CONNECTION_TYPE.into(),
            provider_user_id: None,
            provider_username: Some("acme".into()),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: Some("contents:read".into()),
            expires_at: None,
            installation_id: Some(row.installation_id + 1),
            provider_metadata: None,
        })
        .await
        .unwrap();
    assert_eq!(replacement.id, row.connection_id);
    ctx.db
        .request_agent_source_sync(ctx.org_id(), row.agent_id)
        .await
        .unwrap();
    assert!(reconcile_one(&ctx).await.unwrap());
    let after = binding(&ctx, &id).await.unwrap();
    assert_eq!(after.status, "error");
    assert!(after.error.unwrap().contains("connection changed"));
    assert_eq!(agent(&ctx, &id).await.unwrap().system_prompt, "Good");
}

#[tokio::test]
async fn sync_and_readonly_guard_use_database_identity_when_public_id_differs() {
    let (ctx, remote, old_id) = setup().await;
    let original = agent(&ctx, &old_id).await.unwrap();
    let public = AgentId::new();
    // Client-supplied IDs and older Agents can have independent database/public UUIDs.
    sqlx::query("UPDATE agents SET public_id=$1 WHERE org_id=$2 AND id=$3")
        .bind(public.to_string())
        .bind(ctx.org_id())
        .bind(original.internal_id)
        .execute(ctx.db.pool())
        .await
        .unwrap();
    let id = public.to_string();
    assert_ne!(original.internal_id, public.uuid());
    package_with_skills(&remote, 'a', "First", None, false).await;
    connect(&ctx, &id).await;
    let error = run(
        &ctx,
        "upsert_agent",
        json!({"id":id,"name":"source-agent","system_prompt":"Bypass"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
    remote.reset().await;
    package_with_skills(&remote, 'c', "Second", None, false).await;
    ctx.db
        .request_agent_source_sync(ctx.org_id(), original.internal_id)
        .await
        .unwrap();
    assert!(reconcile_one(&ctx).await.unwrap());
    assert_eq!(binding(&ctx, &id).await.unwrap().error, None);
    assert_eq!(agent(&ctx, &id).await.unwrap().system_prompt, "Second");
    assert_eq!(
        binding(&ctx, &id).await.unwrap().applied_commit,
        Some("c".repeat(40))
    );
}
