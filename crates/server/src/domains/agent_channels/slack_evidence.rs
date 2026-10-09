use crate::domains::agent_channels::ingress::row_to_ingress;
use crate::storage::{EncryptionService, StorageBackend};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) enum DeliveryEvidence {
    WebhookVerified,
    FirstMessage,
}

/// Merge evidence into the current installation, never a webhook's stale copy
/// of its credentials. Serialize with settings, installs, and app removal.
pub(crate) async fn record(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    channel_id: Uuid,
    public_id: &str,
    signing_secret: &str,
    evidence: DeliveryEvidence,
) -> anyhow::Result<()> {
    // Evidence is optional; waiting behind app removal must not consume
    // Slack's three-second acknowledgement window.
    let _guard = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        db.lock_slack_install(channel_id),
    )
    .await??;
    let Some(row) = db.get_ingress_channel_by_public_id(public_id).await? else {
        return Ok(());
    };
    let (context, channel) = row_to_ingress(encryption, row)?;
    let Some(mut config) = channel.slack_config() else {
        return Ok(());
    };
    // A delayed event from a removed/replaced installation is not evidence
    // for the current one. The signature was checked before reaching here.
    if !context.agent_is_active() || config.signing_secret != signing_secret {
        return Ok(());
    }
    let timestamp = match evidence {
        DeliveryEvidence::WebhookVerified => &mut config.webhook_verified_at,
        DeliveryEvidence::FirstMessage => &mut config.first_message_received_at,
    };
    if timestamp.is_some() {
        return Ok(());
    }
    *timestamp = Some(chrono::Utc::now());
    super::queries::update_channel_config_unscoped(
        db,
        encryption,
        channel_id,
        &serde_json::to_value(config)?,
    )
    .await
}
