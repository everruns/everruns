use super::queries::prepare_channel_config;
use crate::domains::agent_channels::record::slack_channel::SlackChannelConfig;
use crate::domains::agent_channels::record::slack_provisioning::SlackProvisioningError;
use crate::domains::common::{CommandError, Ctx};
use crate::storage::{UpdateAgentChannelRow, UpdateField};
use uuid::Uuid;

pub(crate) async fn remove_agent_apps(ctx: &Ctx, agent_id: Uuid) -> Result<(), CommandError> {
    let guard = ctx.db.lock_slack_install(agent_id).await?;
    for channel in ctx.db.list_agent_channels(ctx.org_id(), agent_id).await? {
        if channel.channel_type == "slack" {
            remove_channel_app_locked(ctx, agent_id, &channel.channel_public_id).await?;
        }
    }
    release_after_commit(guard).await;
    Ok(())
}

pub(crate) async fn remove_channel_app(
    ctx: &Ctx,
    agent_id: Uuid,
    channel_id: &str,
) -> Result<(), CommandError> {
    let row = ctx
        .db
        .get_agent_channel(ctx.org_id(), agent_id, channel_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Channel"))?;
    if row.channel_type != "slack" {
        return Ok(());
    }
    let guard = ctx.db.lock_slack_install(agent_id).await?;
    remove_channel_app_locked(ctx, agent_id, channel_id).await?;
    release_after_commit(guard).await;
    Ok(())
}

pub(crate) async fn release_after_commit(guard: crate::storage::backend::SlackInstallLock) {
    // One agent lock covers every channel without pinning a pool connection
    // per channel. Installs wait until the lifecycle transaction commits.
    crate::storage::transaction::after_commit(async move {
        drop(guard);
    })
    .await;
}

async fn remove_channel_app_locked(
    ctx: &Ctx,
    agent_id: Uuid,
    channel_id: &str,
) -> Result<(), CommandError> {
    let row = ctx
        .db
        .get_agent_channel(ctx.org_id(), agent_id, channel_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Channel"))?;
    // THREAT[TM-SLACK-010]: only server-owned install metadata, resolved through
    // the caller's org and agent, may select a Slack app for removal.
    let _channel_guard = ctx.db.lock_slack_install(row.channel_id).await?;
    let row = ctx
        .db
        .get_agent_channel(ctx.org_id(), agent_id, channel_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Channel"))?;
    // Fail closed on unavailable encryption: a null fallback would silently
    // discard the only credentials that can remove this app on a later retry.
    let mut config = match row.channel_config_encrypted.as_deref() {
        Some(bytes) => {
            let encryption = ctx.encryption.as_ref().ok_or_else(|| CommandError::unavailable(
                "Slack connection credentials are unavailable. Restore encryption before retrying."
            ))?;
            serde_json::from_str(&encryption.decrypt_to_string(bytes)?)
                .map_err(|_| CommandError::conflict("Slack connection credentials are invalid. Repair the connection before retrying."))?
        }
        None => row.channel_config,
    };
    let slack: SlackChannelConfig = serde_json::from_value(config.clone()).map_err(|_| {
        CommandError::conflict(
            "Slack connection credentials are invalid. Repair the connection before retrying.",
        )
    })?;
    let managed = slack.provisioned_app.is_some();
    if let Some(app) = slack.provisioned_app {
        // Archiving normally needs manage permission, but deleting a managed
        // Slack identity must not bypass the integration-deletion policy.
        crate::domains::agents::AGENT_DANGEROUS
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .map_err(|_| {
                CommandError::forbidden(
                    "Removing managed Slack apps requires permission to delete Agent integrations.",
                )
            })?;
        let provisioner = ctx.slack_provisioner.as_ref().ok_or_else(|| {
            CommandError::unavailable(
                "Slack app removal is unavailable. Reconnect the Slack workspace before retrying.",
            )
        })?;
        match provisioner
            .delete_app(ctx.org_id(), app.team_id.as_deref(), &app.app_id)
            .await
        {
            Ok(()) => {}
            // External deletion cannot roll back with our command transaction.
            // Treat a previously removed app as success so retries can finish.
            Err(SlackProvisioningError::Rejected(code)) if code == "app_not_found" => {}
            Err(error) => {
                tracing::warn!(channel_id, %error, "Could not remove managed Slack app");
                return Err(CommandError::unavailable(
                    "Could not finish removing the agent from Slack. Some Slack apps may already be removed. Check the connected Slack workspace and retry.",
                ));
            }
        }
        if let Some(config) = config.as_object_mut() {
            for key in [
                "provisioned_app",
                "bot_token",
                "signing_secret",
                "webhook_verified_at",
                "first_message_received_at",
            ] {
                config.remove(key);
            }
        }
    }
    let (config, encrypted) = prepare_channel_config(ctx.encryption.as_ref(), &config)?;
    if managed {
        if !ctx
            .db
            .record_slack_app_removed(ctx.org_id(), agent_id, channel_id, config, encrypted)
            .await?
        {
            return Err(CommandError::not_found("Channel"));
        }
        return Ok(());
    }
    ctx.db
        .update_agent_channel(
            ctx.org_id(),
            agent_id,
            channel_id,
            UpdateAgentChannelRow {
                channel_config: Some(config),
                channel_config_encrypted: encrypted
                    .map(UpdateField::Set)
                    .unwrap_or(UpdateField::Clear),
                enabled: Some(false),
                status: Some("disabled".into()),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}
