// Identity sync belongs to the shared mutation path, including upserts and rollbacks.
// Like avatar propagation, Slack failures never undo a saved agent edit.
use crate::domains::common::Ctx;
use crate::records::Agent;
use crate::records::slack_provisioning::{SlackAppProvisioner, SlackProvisioningError};
use crate::storage::{EncryptionService, IngressChannelRow, StorageBackend};
use std::sync::Arc;
use uuid::Uuid;

// Slack's manifest API quota resets each minute. Tests run against a real
// database, where a paused clock would also fire the pool's acquire timeout,
// so they shorten the wait instead.
#[cfg(not(test))]
const RATE_LIMIT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(60);
#[cfg(test)]
const RATE_LIMIT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

pub(super) fn display_name<'a>(name: &'a str, display_name: Option<&'a str>) -> &'a str {
    display_name.filter(|name| !name.is_empty()).unwrap_or(name)
}

pub(super) fn sync_if_changed(ctx: &Ctx, name: &str, description: Option<&str>, agent: &Agent) {
    let current_name = display_name(&agent.name, agent.display_name.as_deref());
    if (name, description) == (current_name, agent.description.as_deref()) {
        return;
    }
    let Some(provisioner) = ctx.slack_provisioner.clone() else {
        return;
    };
    let (db, encryption, org_id, agent_id) = (
        ctx.db.clone(),
        ctx.encryption.clone(),
        ctx.org_id(),
        agent.internal_id,
    );
    // It reads the agent's channels, so it starts once the change commits.
    crate::storage::transaction::spawn_after_commit(async move {
        let channels = match db.list_agent_channels(org_id, agent_id).await {
            Ok(channels) => channels,
            Err(error) => {
                tracing::warn!(%agent_id, %error, "Could not list endpoints for Slack identity sync");
                return;
            }
        };
        for channel in channels
            .into_iter()
            .filter(|row| row.channel_type == "slack")
        {
            for attempt in 0..3 {
                match sync_channel(
                    &db,
                    encryption.as_ref(),
                    provisioner.as_ref(),
                    org_id,
                    agent_id,
                    &channel,
                )
                .await
                {
                    Ok(()) => break,
                    Err(error) if attempt < 2 && is_rate_limited(&error) => {
                        // Return the lock's connection before waiting for Slack's minute quota.
                        tokio::time::sleep(RATE_LIMIT_RETRY_DELAY).await;
                    }
                    Err(error) => {
                        tracing::warn!(%agent_id, channel_id = %channel.channel_id, %error, "Could not sync agent identity to Slack app");
                        break;
                    }
                }
            }
        }
    });
}

fn is_rate_limited(error: &anyhow::Error) -> bool {
    matches!(error.downcast_ref::<SlackProvisioningError>(), Some(SlackProvisioningError::Rejected(code)) if code == "ratelimited")
}

async fn sync_channel(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    provisioner: &dyn SlackAppProvisioner,
    org_id: i64,
    agent_id: Uuid,
    channel: &IngressChannelRow,
) -> anyhow::Result<()> {
    // Serialize with installs and permission recovery across server instances.
    // Re-read under the lock so queued edits always publish the latest identity.
    // THREAT[TM-SLACK-010]: Resolve the server-owned install state through the owning org and agent.
    let _guard = db.lock_slack_install(channel.channel_id).await?;
    let Some(current) = db
        .get_agent_channel(org_id, agent_id, &channel.channel_public_id)
        .await?
    else {
        return Ok(());
    };
    if current.channel_type != "slack" {
        return Ok(());
    }
    let (_, channel) =
        crate::domains::agent_channels::ingress::row_to_ingress(encryption, current)?;
    let Some(app) = channel.slack_config().and_then(|c| c.provisioned_app) else {
        return Ok(());
    };
    let Some(agent) = db
        .get_agent(
            org_id,
            everruns_contracts::typed_id::AgentId::from_uuid(agent_id),
        )
        .await?
    else {
        return Ok(());
    };
    provisioner
        .update_branding(
            org_id,
            app.team_id.as_deref(),
            &app.app_id,
            display_name(&agent.name, agent.display_name.as_deref()),
            agent.description.as_deref(),
        )
        .await?;
    Ok(())
}
