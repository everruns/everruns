use super::{test_support::*, *};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

// Background tests use the ordinary production feature policy. Bundled Skills
// are tested separately with an explicitly enabled host context, without changing
// process-global rollout configuration or bypassing background permission checks.
async fn package(
    remote: &wiremock::MockServer,
    revision: char,
    instructions: &str,
    capability: Option<&str>,
) {
    test_support::package_with_skills(remote, revision, instructions, capability, false).await;
}

async fn connect_write(ctx: &Ctx, id: &str) {
    let preview = PreviewAgentSource {
        agent_id: id.into(),
        req: location(),
    }
    .run(ctx)
    .await
    .unwrap();
    ConnectAgentSource {
        agent_id: id.into(),
        req: ConnectSourceRequest {
            mode: SourceMode::TwoWay,
            location: location(),
            commit: preview["commit"].as_str().unwrap().into(),
            agent_digest: preview["agent_digest"].as_str().unwrap().into(),
            expected_generation: None,
        },
    }
    .run(ctx)
    .await
    .unwrap();
}

async fn edit(ctx: &Ctx, id: &str) {
    run(
        ctx,
        "update_agent",
        json!({"id":id,"system_prompt":"Edited in Everruns"}),
    )
    .await
    .unwrap();
}

async fn tick(ctx: &Ctx, id: &str) {
    SyncAgentSource {
        agent_id: id.into(),
    }
    .run(ctx)
    .await
    .unwrap();
    assert!(reconcile_one(ctx).await.unwrap());
}

async fn writes(remote: &wiremock::MockServer, patch_status: u16) {
    for (endpoint, sha) in [("git/blobs", 'd'), ("git/trees", 'e'), ("git/commits", 'f')] {
        Mock::given(method("POST"))
            .and(path(format!("/repos/acme/agents/{endpoint}")))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"sha":sha.to_string().repeat(40)})),
            )
            .mount(remote)
            .await;
    }
    Mock::given(method("PATCH"))
        .and(path("/repos/acme/agents/git/refs/heads/main"))
        .respond_with(
            ResponseTemplate::new(patch_status)
                .set_body_json(json!({"object":{"sha":"f".repeat(40)}})),
        )
        .mount(remote)
        .await;
}

async fn prompt(ctx: &Ctx, id: &str) -> String {
    packages::export(ctx, id)
        .await
        .unwrap()
        .manifest
        .instructions
}

#[tokio::test]
async fn local_edits_are_durably_queued_and_written_without_force() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "From GitHub", None).await;
    connect_write(&ctx, &id).await;
    edit(&ctx, &id).await;
    assert!(binding(&ctx, &id).await.unwrap().local_dirty);
    writes(&remote, 200).await;
    let queued = binding(&ctx, &id).await.unwrap();
    ctx.db
        .request_agent_source_sync(ctx.org_id(), queued.agent_id)
        .await
        .unwrap();
    let leased = ctx.db.claim_agent_source_sync().await.unwrap().unwrap();
    write::sync(&ctx, &leased, &id).await.unwrap();
    let row = binding(&ctx, &id).await.unwrap();
    assert_eq!(row.status, "synced", "{:?}", row.error);
    assert_eq!(row.applied_commit, Some("f".repeat(40)));
    assert!(!row.local_dirty);
    assert!(row.outbox.is_none());
    let requests = remote.received_requests().await.unwrap();
    let patch = requests
        .iter()
        .find(|r| r.method.as_str() == "PATCH")
        .unwrap();
    assert_eq!(
        patch.body_json::<Value>().unwrap(),
        json!({"sha":"f".repeat(40),"force":false})
    );
    let tree = requests
        .iter()
        .find(|r| r.method.as_str() == "POST" && r.url.path().ends_with("/git/trees"))
        .unwrap();
    let tree = tree.body_json::<Value>().unwrap();
    assert_eq!(tree["base_tree"], "b".repeat(40));
    assert_eq!(tree["tree"].as_array().unwrap().len(), 1);
    assert_eq!(tree["tree"][0]["path"], "instructions.md");
    assert_eq!(prompt(&ctx, &id).await, "Edited in Everruns");
}

#[tokio::test]
async fn simultaneous_package_changes_pause_without_overwriting_either_side() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "From GitHub", None).await;
    connect_write(&ctx, &id).await;
    edit(&ctx, &id).await;
    remote.reset().await;
    package(&remote, 'c', "Edited in GitHub", None).await;
    tick(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    assert_eq!(row.status, "conflict");
    assert_eq!(row.applied_commit, Some("a".repeat(40)));
    assert_eq!(prompt(&ctx, &id).await, "Edited in Everruns");
    assert!(
        remote
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET")
    );
    // Reviewing the remote version explicitly resolves a conflict without a force push.
    let preview = PreviewAgentSource {
        agent_id: id.clone(),
        req: location(),
    }
    .run(&ctx)
    .await
    .unwrap();
    ConnectAgentSource {
        agent_id: id.clone(),
        req: ConnectSourceRequest {
            mode: SourceMode::TwoWay,
            location: location(),
            commit: "c".repeat(40),
            agent_digest: preview["agent_digest"].as_str().unwrap().into(),
            expected_generation: Some(row.generation),
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(prompt(&ctx, &id).await, "Edited in GitHub");
}

#[tokio::test]
async fn protected_branch_failure_keeps_local_edits_and_reuses_the_persisted_commit() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "From GitHub", None).await;
    connect_write(&ctx, &id).await;
    edit(&ctx, &id).await;
    writes(&remote, 403).await;
    tick(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    assert_eq!(row.status, "error");
    assert!(row.outbox.is_some(), "{:?}", row.error);
    assert_eq!(prompt(&ctx, &id).await, "Edited in Everruns");
    // Simulate a lost success response: GitHub already points at the staged commit.
    remote.reset().await;
    get(&remote, "/repos/acme/agents", json!({"id":123})).await;
    get(
        &remote,
        "/repos/acme/agents/git/ref/heads/main",
        json!({"object":{"sha":"f".repeat(40)}}),
    )
    .await;
    get(
        &remote,
        &format!("/repos/acme/agents/git/commits/{}", "f".repeat(40)),
        json!({"tree":{"sha":"e".repeat(40)}}),
    )
    .await;
    tick(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    assert_eq!(row.status, "synced", "{:?}", row.error);
    assert!(row.outbox.is_none());
    assert!(
        remote
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET")
    );
}

#[tokio::test]
async fn two_way_sync_pulls_remote_edits_and_preserves_unrelated_branch_commits_on_push() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "From GitHub", None).await;
    connect_write(&ctx, &id).await;
    remote.reset().await;
    package(&remote, 'c', "Updated remotely", None).await;
    tick(&ctx, &id).await;
    let status = binding(&ctx, &id).await.unwrap();
    assert_eq!(status.status, "synced", "{:?}", status.error);
    assert_eq!(prompt(&ctx, &id).await, "Updated remotely");
    edit(&ctx, &id).await;
    remote.reset().await;
    // The branch advances, but the selected package is semantically unchanged.
    package(&remote, 'd', "Updated remotely", None).await;
    writes(&remote, 200).await;
    tick(&ctx, &id).await;
    assert_eq!(binding(&ctx, &id).await.unwrap().status, "synced");
    let requests = remote.received_requests().await.unwrap();
    let commit = requests
        .iter()
        .find(|r| r.method.as_str() == "POST" && r.url.path().ends_with("/git/commits"))
        .unwrap();
    assert_eq!(
        commit.body_json::<Value>().unwrap()["parents"],
        json!(["d".repeat(40)])
    );
}

#[tokio::test]
async fn disconnect_fences_a_prepared_write() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "From GitHub", None).await;
    connect_write(&ctx, &id).await;
    edit(&ctx, &id).await;
    writes(&remote, 403).await;
    tick(&ctx, &id).await;
    let old = binding(&ctx, &id).await.unwrap();
    DisconnectAgentSource {
        agent_id: id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap();
    remote.reset().await;
    write::sync(&ctx, &old, &id).await.unwrap();
    assert!(remote.received_requests().await.unwrap().is_empty());
    assert_eq!(prompt(&ctx, &id).await, "Edited in Everruns");
}

#[tokio::test]
async fn a_later_local_edit_survives_delivery_of_an_older_queued_snapshot() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "From GitHub", None).await;
    connect_write(&ctx, &id).await;
    edit(&ctx, &id).await;
    writes(&remote, 403).await;
    tick(&ctx, &id).await;
    assert!(binding(&ctx, &id).await.unwrap().outbox.is_some());
    run(
        &ctx,
        "update_agent",
        json!({"id":id,"system_prompt":"Even newer local edit"}),
    )
    .await
    .unwrap();
    remote.reset().await;
    get(&remote, "/repos/acme/agents", json!({"id":123})).await;
    get(
        &remote,
        "/repos/acme/agents/git/ref/heads/main",
        json!({"object":{"sha":"f".repeat(40)}}),
    )
    .await;
    get(
        &remote,
        &format!("/repos/acme/agents/git/commits/{}", "f".repeat(40)),
        json!({"tree":{"sha":"e".repeat(40)}}),
    )
    .await;
    tick(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    assert!(row.outbox.is_none());
    assert!(row.local_dirty);
    assert_eq!(row.status, "pending");
    assert_eq!(prompt(&ctx, &id).await, "Even newer local edit");
}

#[tokio::test]
async fn unnamed_local_channels_are_not_published() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "From GitHub", None).await;
    connect_write(&ctx, &id).await;
    run(
        &ctx,
        "create_agent_channel",
        json!({"agent_id":id,"channel_type":"ag_ui","channel_config":{}}),
    )
    .await
    .unwrap();
    tick(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    assert_eq!(row.status, "synced", "{:?}", row.error);
    assert!(row.outbox.is_none());
    assert!(
        remote
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET")
    );
}

#[tokio::test]
async fn a_local_package_import_can_explicitly_add_a_source_owned_channel() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "From GitHub", None).await;
    connect_write(&ctx, &id).await;
    let mut desired = write::local_package(&ctx, &id).await.unwrap();
    desired.manifest.channels.insert(
        "another".into(),
        everruns_core::agent_package::Channel {
            channel_type: "ag_ui".into(),
            enabled: false,
            config: json!({}),
        },
    );
    packages::ImportAgent::Package(packages::PackageInput {
        content: desired.to_string(Format::Json).unwrap(),
        file: None,
        format: Some("json".into()),
        target: Some(id.clone()),
    })
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(
        binding(&ctx, &id).await.unwrap().managed_channels,
        vec!["another", "chat"]
    );
    writes(&remote, 200).await;
    tick(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    assert_eq!(row.status, "synced", "{:?}", row.error);
    let requests = remote.received_requests().await.unwrap();
    let blob = requests
        .iter()
        .find(|r| r.method.as_str() == "POST" && r.url.path().ends_with("/git/blobs"))
        .unwrap()
        .body_json::<Value>()
        .unwrap();
    let bytes = STANDARD.decode(blob["content"].as_str().unwrap()).unwrap();
    let manifest = AgentPackage::parse(std::str::from_utf8(&bytes).unwrap(), Format::Toml).unwrap();
    assert!(manifest.manifest.channels.contains_key("another"));
}

#[tokio::test]
async fn omitted_named_channel_bindings_stay_local_after_a_remote_update() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "From GitHub", None).await;
    connect_write(&ctx, &id).await;
    remote.reset().await;
    package(&remote, 'c', "Updated remotely", None).await;
    let text = "schema_version=1\nname='source-agent'\nharness='base'\nfiles=['data/**']\n";
    Mock::given(method("GET"))
        .and(path(format!("/repos/acme/agents/git/blobs/{:040x}", 1)))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"encoding":"base64","content":STANDARD.encode(text)})),
        )
        .with_priority(1)
        .mount(&remote)
        .await;
    tick(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    assert_eq!(row.status, "synced", "{:?}", row.error);
    assert!(row.managed_channels.is_empty());
    let channels = crate::domains::agent_channels::ListAgentChannels {
        agent_id: id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(channels.len(), 1);
    let mut config = channels[0].channel_config.clone();
    config["rate_limit_per_minute"] = json!(7);
    run(
        &ctx,
        "update_agent_channel",
        json!({"agent_id":id,"channel_id":channels[0].public_id,"channel_config":config}),
    )
    .await
    .unwrap();
    tick(&ctx, &id).await;
    assert!(
        remote
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET")
    );
}

#[tokio::test]
async fn idle_two_way_checks_do_not_download_package_blobs() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "From GitHub", None).await;
    connect_write(&ctx, &id).await;
    remote.reset().await;
    get(&remote, "/repos/acme/agents", json!({"id":123})).await;
    get(
        &remote,
        "/repos/acme/agents/git/ref/heads/main",
        json!({"object":{"sha":"a".repeat(40)}}),
    )
    .await;
    tick(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    assert_eq!(row.status, "synced", "{:?}", row.error);
    assert_eq!(remote.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn enabled_skills_are_preserved_without_internal_caller_privileges() {
    let (mut ctx, remote, id) = setup().await;
    ctx.feature_flags.skills = true;
    ctx.caller.is_internal = false;
    ctx.caller.is_platform_user = false;
    test_support::package_with_skills(&remote, 'a', "With a bundled skill", None, true).await;
    connect_write(&ctx, &id).await;
    edit(&ctx, &id).await;
    writes(&remote, 200).await;
    let row = ctx.db.claim_agent_source_sync().await.unwrap().unwrap();
    write::sync(&ctx, &row, &id).await.unwrap();
    assert_eq!(binding(&ctx, &id).await.unwrap().status, "synced");
    let files = write::local_package(&ctx, &id)
        .await
        .unwrap()
        .files()
        .unwrap();
    assert!(
        files
            .iter()
            .any(|file| file.path == "/.agents/skills/helper/SKILL.md")
    );
    let requests = remote.received_requests().await.unwrap();
    let changes = requests
        .iter()
        .find(|r| r.method.as_str() == "POST" && r.url.path().ends_with("/git/trees"))
        .unwrap()
        .body_json::<Value>()
        .unwrap();
    assert_eq!(changes["tree"].as_array().unwrap().len(), 1);
    assert_eq!(changes["tree"][0]["path"], "instructions.md");
}

#[tokio::test]
async fn queued_writes_recheck_feature_policy_before_retrying_a_ref_update() {
    let (mut ctx, remote, id) = setup().await;
    ctx.feature_flags.skills = true;
    ctx.caller.is_internal = false;
    ctx.caller.is_platform_user = false;
    test_support::package_with_skills(&remote, 'a', "With a bundled skill", None, true).await;
    connect_write(&ctx, &id).await;
    edit(&ctx, &id).await;
    writes(&remote, 403).await;
    let row = ctx.db.claim_agent_source_sync().await.unwrap().unwrap();
    assert!(write::sync(&ctx, &row, &id).await.is_err());
    let queued = binding(&ctx, &id).await.unwrap();
    assert!(queued.outbox.is_some());
    ctx.feature_flags.skills = false;
    let error = write::sync(&ctx, &queued, &id).await.unwrap_err();
    assert!(error.message().contains("Queued edits no longer satisfy"));
    let requests = remote.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method.as_str() == "PATCH")
            .count(),
        1
    );
}

#[tokio::test]
async fn ambiguous_source_channel_bindings_cannot_overwrite_repository_intent() {
    let (ctx, remote, id) = setup().await;
    package(&remote, 'a', "From GitHub", None).await;
    connect_write(&ctx, &id).await;
    let result=run(&ctx,"create_agent_channel",json!({"agent_id":id,"channel_type":"ag_ui","channel_config":{"package_name":"chat","rate_limit_per_minute":7}})).await;
    if result.is_err() {
        return;
    } // Rejecting duplicates at creation is safe too.
    tick(&ctx, &id).await;
    let row = binding(&ctx, &id).await.unwrap();
    assert_eq!(row.status, "error");
    assert!(row.error.unwrap().contains("Multiple channels claim"));
    assert!(
        remote
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET")
    );
}
