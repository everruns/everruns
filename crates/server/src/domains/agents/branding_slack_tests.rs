use super::*;
use crate::records::slack_provisioning::*;
use crate::storage::CreateAgentChannelRow;
use serde_json::json;
use std::sync::Mutex;

type Identity = (i64, String, String, String, Option<String>);
#[derive(Default)]
struct Provisioner {
    identities: Mutex<Vec<Identity>>,
    rate_attempts: std::sync::atomic::AtomicUsize,
}
#[async_trait]
impl SlackAppProvisioner for Provisioner {
    async fn update_branding(
        &self,
        org: i64,
        team: Option<&str>,
        app: &str,
        name: &str,
        description: Option<&str>,
    ) -> SlackProvisioningResult<()> {
        if app == "A-rate"
            && self
                .rate_attempts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                == 0
        {
            return Err(SlackProvisioningError::Rejected("ratelimited".into()));
        }
        if app == "A-fail" {
            return Err(SlackProvisioningError::Rejected("invalid_auth".into()));
        }
        self.identities.lock().unwrap().push((
            org,
            team.unwrap().into(),
            app.into(),
            name.into(),
            description.map(str::to_owned),
        ));
        Ok(())
    }
    async fn create_app(
        &self,
        _: i64,
        _: Option<&str>,
        _: &str,
    ) -> SlackProvisioningResult<SlackAppCredentials> {
        unreachable!()
    }
    async fn delete_app(&self, _: i64, _: Option<&str>, _: &str) -> SlackProvisioningResult<()> {
        unreachable!()
    }
    async fn connection_status(
        &self,
        _: i64,
    ) -> SlackProvisioningResult<SlackProvisioningConnectionStatus> {
        unreachable!()
    }
}
async fn endpoint(ctx: &Ctx, agent: &Agent, app: Option<&str>, team: &str) -> Uuid {
    let config = match app {
        Some(app) => {
            json!({"provisioned_app":{"app_id":app,"client_id":"client","client_secret":"secret","team_id":team}})
        }
        None => json!({}),
    };
    ctx.db
        .create_agent_channel(
            ctx.org_id(),
            CreateAgentChannelRow {
                agent_id: agent.internal_id,
                public_id: format!("channel_{}", Uuid::now_v7().simple()),
                channel_type: "slack".into(),
                channel_config: config,
                channel_config_encrypted: None,
                auth: None,
                auth_encrypted: None,
                enabled: true,
                status: "published".into(),
                virtual_user_id: None,
                agent_version_policy: "latest".into(),
                agent_version_id: None,
                owner_principal_id: Uuid::nil(),
                resolved_owner_user_id: None,
            },
        )
        .await
        .unwrap()
        .channel_id
}
async fn wait_for(provisioner: &Provisioner, count: usize) -> Vec<Identity> {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let identities = provisioner.identities.lock().unwrap().clone();
            if identities.len() >= count {
                return identities;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("saved identity reaches managed Slack apps")
}

#[tokio::test]
async fn branding_update_is_scoped_and_failure_does_not_block_other_apps() {
    let db = Arc::new(StorageBackend::in_memory());
    let provisioner = Arc::new(Provisioner::default());
    let ctx =
        ctx_with_role(db.clone(), OrgRole::Owner).with_slack_provisioner(Some(provisioner.clone()));
    let agent = CreateAgent(basic_agent_request("branding-agent"))
        .run(&ctx)
        .await
        .unwrap();
    endpoint(&ctx, &agent, Some("A-fail"), "T1").await;
    endpoint(&ctx, &agent, Some("A1"), "T1").await;
    endpoint(&ctx, &agent, Some("A2"), "T2").await;
    endpoint(&ctx, &agent, None, "manual").await;
    let other = CreateAgent(basic_agent_request("other-agent"))
        .run(&ctx)
        .await
        .unwrap();
    endpoint(&ctx, &other, Some("A-other"), "T1").await;
    crate::org_init::initialize_org_harnesses(&db, 2)
        .await
        .unwrap();
    let mut other_ctx = ctx.clone();
    other_ctx.caller.org_id = 2;
    let foreign = CreateAgent(basic_agent_request("foreign-agent"))
        .run(&other_ctx)
        .await
        .unwrap();
    endpoint(&other_ctx, &foreign, Some("A-foreign"), "T1").await;
    let updated = UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            display_name: Some("New display name".into()),
            description: Some("New description".into()),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(updated.display_name.as_deref(), Some("New display name"));
    let identities = wait_for(&provisioner, 2).await;
    assert_eq!(
        identities,
        vec![
            (
                DEFAULT_ORG_ID,
                "T1".into(),
                "A1".into(),
                "New display name".into(),
                Some("New description".into())
            ),
            (
                DEFAULT_ORG_ID,
                "T2".into(),
                "A2".into(),
                "New display name".into(),
                Some("New description".into())
            ),
        ]
    );
    UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            system_prompt: Some("Unrelated change".into()),
            name: Some("new-slug".into()),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert_eq!(
        *provisioner.identities.lock().unwrap(),
        identities,
        "unchanged effective identity does not consume Slack quota"
    );
}

#[tokio::test]
async fn branding_upsert_uses_name_fallback_and_rollback_restores_identity() {
    let db = Arc::new(StorageBackend::in_memory());
    let provisioner = Arc::new(Provisioner::default());
    let ctx = ctx_with_role(db, OrgRole::Owner).with_slack_provisioner(Some(provisioner.clone()));
    let mut req = basic_agent_request("original-slug");
    req.display_name = Some("Original display".into());
    req.description = Some("Original description".into());
    let agent = CreateAgent(req).run(&ctx).await.unwrap();
    endpoint(&ctx, &agent, Some("A1"), "T1").await;
    let version = CreateAgentVersionCmd {
        agent_id: agent.public_id.to_string(),
        req: CreateAgentVersionRequest {
            summary: None,
            change_kind: None,
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    UpsertAgent {
        replace_capabilities: false,
        id: agent.public_id.to_string(),
        req: basic_agent_request("new-slug"),
    }
    .run(&ctx)
    .await
    .unwrap();
    let identities = wait_for(&provisioner, 1).await;
    assert_eq!(identities[0].3, "new-slug");
    assert_eq!(identities[0].4, None);
    RollbackAgentVersion {
        agent_id: agent.public_id.to_string(),
        version_id: version.public_id,
        req: RollbackAgentVersionRequest {
            save_version: false,
            summary: None,
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    let identities = wait_for(&provisioner, 2).await;
    assert_eq!(identities[1].3, "Original display");
    assert_eq!(identities[1].4.as_deref(), Some("Original description"));
}

#[tokio::test]
async fn branding_cleared_display_name_falls_back_for_sync_and_new_install() {
    let db = Arc::new(StorageBackend::in_memory());
    let provisioner = Arc::new(Provisioner::default());
    let ctx =
        ctx_with_role(db.clone(), OrgRole::Owner).with_slack_provisioner(Some(provisioner.clone()));
    let mut request = basic_agent_request("fallback-slug");
    request.display_name = Some("Before".into());
    let agent = CreateAgent(request).run(&ctx).await.unwrap();
    endpoint(&ctx, &agent, Some("A1"), "T1").await;
    let cleared = UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            display_name: Some(String::new()),
            description: Some(String::new()),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    let identities = wait_for(&provisioner, 1).await;
    assert_eq!(identities[0].3, "fallback-slug");
    let new_endpoint = endpoint(&ctx, &cleared, None, "T1").await;
    let row = db
        .list_agent_channels(ctx.org_id(), agent.internal_id)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.channel_id == new_endpoint)
        .unwrap();
    assert_eq!(
        row.agent_name, "fallback-slug",
        "new Slack installs use the same fallback as edits"
    );
}

#[tokio::test]
async fn branding_queued_updates_read_latest_saved_identity_under_install_lock() {
    let db = Arc::new(StorageBackend::in_memory());
    let provisioner = Arc::new(Provisioner::default());
    let ctx =
        ctx_with_role(db.clone(), OrgRole::Owner).with_slack_provisioner(Some(provisioner.clone()));
    let agent = CreateAgent(basic_agent_request("queued-agent"))
        .run(&ctx)
        .await
        .unwrap();
    let endpoint = endpoint(&ctx, &agent, Some("A1"), "T1").await;
    let guard = db.lock_slack_install(endpoint).await.unwrap();
    for title in ["Intermediate", "Latest"] {
        UpdateAgentCmd {
            id: agent.public_id.to_string(),
            req: UpdateAgentRequest {
                display_name: Some(title.into()),
                ..Default::default()
            },
        }
        .run(&ctx)
        .await
        .unwrap();
    }
    drop(guard);
    let identities = wait_for(&provisioner, 2).await;
    assert!(identities.iter().all(|identity| identity.3 == "Latest"));
}

#[tokio::test(start_paused = true)]
async fn branding_rate_limit_retry_releases_lock_and_reads_latest_identity() {
    let db = Arc::new(StorageBackend::in_memory());
    let provisioner = Arc::new(Provisioner::default());
    let ctx =
        ctx_with_role(db.clone(), OrgRole::Owner).with_slack_provisioner(Some(provisioner.clone()));
    let agent = CreateAgent(basic_agent_request("rate-agent"))
        .run(&ctx)
        .await
        .unwrap();
    let endpoint = endpoint(&ctx, &agent, Some("A-rate"), "T1").await;
    UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            display_name: Some("Intermediate".into()),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    for _ in 0..100 {
        if provisioner
            .rate_attempts
            .load(std::sync::atomic::Ordering::SeqCst)
            > 0
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        provisioner
            .rate_attempts
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "first request is rate-limited"
    );
    let guard = db.lock_slack_install(endpoint).await.unwrap();
    db.update_agent(
        ctx.org_id(),
        everruns_contracts::typed_id::AgentId::from_uuid(agent.internal_id),
        UpdateAgent {
            display_name: Some("Latest".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    drop(guard);
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    let identities = wait_for(&provisioner, 1).await;
    assert_eq!(identities[0].3, "Latest");
    assert_eq!(
        provisioner
            .rate_attempts
            .load(std::sync::atomic::Ordering::SeqCst),
        2
    );
}
