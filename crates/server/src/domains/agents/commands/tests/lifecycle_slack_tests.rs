use super::*;
use crate::domains::agent_channels::{DeleteAgentChannel, ListAgentChannels};
use crate::records::slack_provisioning::*;
use crate::storage::CreateAgentChannelRow;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Mutex;

#[derive(Default)]
struct Provisioner {
    apps: Mutex<HashSet<(i64, String, String)>>,
    fail: Mutex<bool>,
    fail_app: Mutex<Option<String>>,
}

#[async_trait]
impl SlackAppProvisioner for Provisioner {
    async fn create_app(
        &self,
        _: i64,
        _: Option<&str>,
        _: &str,
    ) -> SlackProvisioningResult<SlackAppCredentials> {
        unreachable!()
    }
    async fn delete_app(
        &self,
        org: i64,
        team: Option<&str>,
        app: &str,
    ) -> SlackProvisioningResult<()> {
        if *self.fail.lock().unwrap() || self.fail_app.lock().unwrap().as_deref() == Some(app) {
            return Err(SlackProvisioningError::Rejected("ratelimited".into()));
        }
        if !self
            .apps
            .lock()
            .unwrap()
            .remove(&(org, team.unwrap().into(), app.into()))
        {
            return Err(SlackProvisioningError::Rejected("app_not_found".into()));
        }
        Ok(())
    }
    async fn connection_status(
        &self,
        _: i64,
    ) -> SlackProvisioningResult<SlackProvisioningConnectionStatus> {
        unreachable!()
    }
}

async fn endpoint(ctx: &Ctx, agent: &Agent, app: Option<&str>, encrypted: bool) -> String {
    let mut config = json!({"bot_token":"saved-bot-token", "signing_secret":"saved-signing-secret", "team_id":"T1"});
    if let Some(app) = app {
        config["provisioned_app"] =
            json!({"app_id":app,"client_id":"client","client_secret":"secret","team_id":"T1"});
    }
    let (config, ciphertext) = crate::domains::agent_channels::queries::prepare_channel_config(
        if encrypted {
            ctx.encryption.as_ref()
        } else {
            None
        },
        &config,
    )
    .unwrap();
    let public_id = format!("appchan_{}", Uuid::now_v7().simple());
    ctx.db
        .create_agent_channel(
            ctx.org_id(),
            CreateAgentChannelRow {
                agent_id: agent.internal_id,
                public_id: public_id.clone(),
                channel_type: "slack".into(),
                channel_config: config,
                channel_config_encrypted: ciphertext,
                auth: None,
                auth_encrypted: None,
                enabled: true,
                status: "live".into(),
                virtual_user_id: None,
                owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1).uuid(),
                resolved_owner_user_id: None,
            },
        )
        .await
        .unwrap();
    public_id
}

async fn setup() -> (Ctx, Agent, Arc<Provisioner>) {
    let db = Arc::new(StorageBackend::test_database());
    let provisioner = Arc::new(Provisioner::default());
    let mut ctx = ctx_with_role(db, OrgRole::Owner)
        .await
        .with_slack_provisioner(Some(provisioner.clone()));
    ctx.encryption = Some(Arc::new(
        crate::storage::EncryptionService::new(
            "v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            &[],
        )
        .unwrap(),
    ));
    let agent = CreateAgent(basic_agent_request("slack-lifecycle"))
        .run(&ctx)
        .await
        .unwrap();
    for app in ["A1", "A2", "A-other"] {
        provisioner
            .apps
            .lock()
            .unwrap()
            .insert((ctx.org_id(), "T1".into(), app.into()));
    }
    (ctx, agent, provisioner)
}

#[tokio::test]
async fn archive_removes_owned_slack_apps_and_restore_requires_reinstall() {
    let (ctx, agent, provisioner) = setup().await;
    endpoint(&ctx, &agent, Some("A1"), true).await;
    endpoint(&ctx, &agent, Some("A2"), false).await;
    endpoint(&ctx, &agent, None, false).await;
    DeleteAgent {
        id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(provisioner.apps.lock().unwrap().len(), 1);
    let channels = ListAgentChannels {
        agent_id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(channels.len(), 3);
    for channel in channels {
        assert!(!channel.enabled);
        assert_eq!(channel.status, crate::records::ChannelStatus::Disabled);
        assert!(channel.channel_config.get("provisioned_app").is_none());
    }
    ctx.db
        .update_agent(
            ctx.org_id(),
            AgentId::from_uuid(agent.internal_id),
            crate::storage::UpdateAgent {
                status: Some("active".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let channels = ctx
        .db
        .list_agent_channels(ctx.org_id(), agent.internal_id)
        .await
        .unwrap();
    assert!(channels.iter().all(|c| c.channel_status == "disabled"));
    let (_, channel) = crate::domains::agent_channels::ingress::row_to_ingress(
        ctx.encryption.as_ref(),
        channels[0].clone(),
    )
    .unwrap();
    assert!(channel.slack_config().unwrap().bot_token.is_empty());
}

#[tokio::test]
async fn channel_delete_removes_only_its_managed_slack_app() {
    let (ctx, agent, provisioner) = setup().await;
    let channel_id = endpoint(&ctx, &agent, Some("A1"), true).await;
    DeleteAgentChannel {
        agent_id: agent.public_id.to_string(),
        channel_id: channel_id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(provisioner.apps.lock().unwrap().len(), 2);
    assert!(
        ctx.db
            .get_agent_channel(ctx.org_id(), agent.internal_id, &channel_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn cleanup_failure_preserves_agent_and_install_credentials_for_retry() {
    let (ctx, agent, provisioner) = setup().await;
    let channel_id = endpoint(&ctx, &agent, Some("A1"), true).await;
    *provisioner.fail.lock().unwrap() = true;
    let error = DeleteAgent {
        id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert!(error.message().contains("Slack"));
    assert_eq!(
        ctx.db
            .get_agent(ctx.org_id(), AgentId::from_uuid(agent.internal_id))
            .await
            .unwrap()
            .unwrap()
            .status,
        "active"
    );
    let row = ctx
        .db
        .get_agent_channel(ctx.org_id(), agent.internal_id, &channel_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        crate::domains::agent_channels::ingress::row_to_ingress(ctx.encryption.as_ref(), row)
            .unwrap()
            .1
            .slack_config()
            .unwrap()
            .provisioned_app
            .is_some()
    );
    *provisioner.fail.lock().unwrap() = false;
    DeleteAgent {
        id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(provisioner.apps.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn delete_cleans_up_apps_on_agents_archived_before_cleanup_existed() {
    let (ctx, agent, provisioner) = setup().await;
    endpoint(&ctx, &agent, Some("A1"), false).await;
    ctx.db
        .delete_agent(ctx.org_id(), AgentId::from_uuid(agent.internal_id))
        .await
        .unwrap();
    DestroyAgent {
        id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(provisioner.apps.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn already_deleted_slack_app_does_not_trap_agent_lifecycle() {
    let (ctx, agent, provisioner) = setup().await;
    endpoint(&ctx, &agent, Some("A-gone"), false).await;
    DeleteAgent {
        id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(provisioner.apps.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn partial_external_cleanup_keeps_progress_when_agent_archive_rolls_back() {
    let (ctx, agent, provisioner) = setup().await;
    endpoint(&ctx, &agent, Some("A1"), false).await;
    endpoint(&ctx, &agent, Some("A2"), false).await;
    *provisioner.fail_app.lock().unwrap() = Some("A2".into());
    DeleteAgent {
        id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert_eq!(provisioner.apps.lock().unwrap().len(), 2);
    assert_eq!(
        ctx.db
            .get_agent(ctx.org_id(), AgentId::from_uuid(agent.internal_id))
            .await
            .unwrap()
            .unwrap()
            .status,
        "active"
    );
    let channels = ctx
        .db
        .list_agent_channels(ctx.org_id(), agent.internal_id)
        .await
        .unwrap();
    assert!(channels[0].channel_config.get("provisioned_app").is_none());
    assert_eq!(channels[0].channel_status, "disabled");
    *provisioner.fail_app.lock().unwrap() = None;
    DeleteAgent {
        id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(provisioner.apps.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn patch_archive_uses_the_same_slack_cleanup() {
    let (ctx, agent, provisioner) = setup().await;
    endpoint(&ctx, &agent, Some("A1"), false).await;
    UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            status: Some(AgentStatus::Archived),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(provisioner.apps.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn queued_channel_edit_cannot_restore_removed_slack_credentials() {
    for archived in [false, true] {
        let (ctx, agent, _) = setup().await;
        let channel_id = endpoint(&ctx, &agent, Some("A1"), true).await;
        let row = ctx
            .db
            .get_agent_channel(ctx.org_id(), agent.internal_id, &channel_id)
            .await
            .unwrap()
            .unwrap();
        let guard = ctx.db.lock_slack_install(row.channel_id).await.unwrap();
        let edit_ctx = ctx.clone();
        let public_agent_id = agent.public_id.to_string();
        let edit_channel_id = channel_id.clone();
        let mut edit = tokio::spawn(async move {
            crate::domains::agent_channels::UpdateAgentChannelCmd {
                agent_id: public_agent_id,
                channel_id: edit_channel_id,
                req: crate::domains::agent_channels::types::UpdateAgentChannelRequest {
                    channel_config: Some(json!({"reply_mode":"tool_only"})),
                    ..Default::default()
                },
            }
            .run(&edit_ctx)
            .await
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut edit)
                .await
                .is_err()
        );
        // Model cleanup's durable credential clear and the archive commit while
        // its channel lock is still held; the queued edit must re-read afterwards.
        ctx.db
            .record_slack_app_removed(
                ctx.org_id(),
                agent.internal_id,
                &channel_id,
                json!({}),
                None,
            )
            .await
            .unwrap();
        if archived {
            ctx.db
                .delete_agent(ctx.org_id(), AgentId::from_uuid(agent.internal_id))
                .await
                .unwrap();
        }
        drop(guard);
        assert_eq!(edit.await.unwrap().is_err(), archived);
        let row = ctx
            .db
            .get_agent_channel(ctx.org_id(), agent.internal_id, &channel_id)
            .await
            .unwrap()
            .unwrap();
        let config = crate::domains::agent_channels::ingress::row_to_ingress(
            ctx.encryption.as_ref(),
            row.clone(),
        )
        .unwrap()
        .1
        .slack_config()
        .unwrap();
        assert!(config.provisioned_app.is_none());
        assert!(config.bot_token.is_empty());
        assert_eq!(row.channel_status, "disabled");
    }
}

#[tokio::test]
async fn delayed_slack_delivery_evidence_cannot_restore_removed_credentials() {
    let (ctx, agent, provisioner) = setup().await;
    let channel_id = endpoint(&ctx, &agent, Some("A1"), true).await;
    let row = ctx
        .db
        .get_agent_channel(ctx.org_id(), agent.internal_id, &channel_id)
        .await
        .unwrap()
        .unwrap();
    endpoint(&ctx, &agent, Some("A2"), false).await;
    *provisioner.fail_app.lock().unwrap() = Some("A2".into());
    DeleteAgent {
        id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap_err();
    for evidence in [
        crate::domains::agent_channels::slack_evidence::DeliveryEvidence::WebhookVerified,
        crate::domains::agent_channels::slack_evidence::DeliveryEvidence::FirstMessage,
    ] {
        crate::domains::agent_channels::slack_evidence::record(
            &ctx.db,
            ctx.encryption.as_ref(),
            row.channel_id,
            &channel_id,
            "saved-signing-secret",
            evidence,
        )
        .await
        .unwrap();
    }
    let row = ctx
        .db
        .get_agent_channel(ctx.org_id(), agent.internal_id, &channel_id)
        .await
        .unwrap()
        .unwrap();
    let config =
        crate::domains::agent_channels::ingress::row_to_ingress(ctx.encryption.as_ref(), row)
            .unwrap()
            .1
            .slack_config()
            .unwrap();
    assert!(config.provisioned_app.is_none());
    assert!(config.bot_token.is_empty());
    assert!(config.webhook_verified_at.is_none());
    assert!(config.first_message_received_at.is_none());
}

#[tokio::test]
async fn archive_cannot_bypass_managed_slack_app_deletion_permission() {
    for managed in [false, true] {
        for patch in [false, true] {
            let (ctx, agent, provisioner) = setup().await;
            endpoint(&ctx, &agent, managed.then_some("A1"), true).await;
            let mut member = ctx_with_role(ctx.db.clone(), OrgRole::Member)
                .await
                .with_slack_provisioner(Some(provisioner.clone()));
            member.encryption = ctx.encryption.clone();
            let result = if patch {
                UpdateAgentCmd {
                    id: agent.public_id.to_string(),
                    req: UpdateAgentRequest {
                        status: Some(AgentStatus::Archived),
                        ..Default::default()
                    },
                }
                .run(&member)
                .await
                .map(|_| ())
            } else {
                DeleteAgent {
                    id: agent.public_id.to_string(),
                }
                .run(&member)
                .await
                .map(|_| ())
            };
            if managed {
                assert_eq!(
                    result.unwrap_err().status(),
                    axum::http::StatusCode::FORBIDDEN
                );
            } else {
                result.unwrap();
            }
            assert_eq!(provisioner.apps.lock().unwrap().len(), 3);
            let row = ctx
                .db
                .get_agent(ctx.org_id(), AgentId::from_uuid(agent.internal_id))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, if managed { "active" } else { "archived" });
        }
    }
}

#[tokio::test]
async fn invalid_patch_does_not_remove_slack_apps() {
    let (ctx, agent, provisioner) = setup().await;
    endpoint(&ctx, &agent, Some("A1"), false).await;
    UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            status: Some(AgentStatus::Archived),
            max_iterations: Some(i32::MAX as usize + 1),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert_eq!(provisioner.apps.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn missing_encryption_and_cross_org_requests_cannot_discard_or_remove_an_app() {
    let (mut ctx, agent, provisioner) = setup().await;
    let channel_id = endpoint(&ctx, &agent, Some("A1"), true).await;
    ctx.encryption = None;
    DeleteAgent {
        id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert_eq!(provisioner.apps.lock().unwrap().len(), 3);
    ctx.caller.org_id = 2;
    DeleteAgentChannel {
        agent_id: agent.public_id.to_string(),
        channel_id,
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert_eq!(provisioner.apps.lock().unwrap().len(), 3);
}
