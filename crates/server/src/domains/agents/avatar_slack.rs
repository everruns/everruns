// Agent avatar -> Slack app icon.
//
// Each Slack endpoint is its own Slack app, and what people see next to the
// agent's messages is that app's icon. A manifest cannot carry an icon, so the
// avatar is pushed with `apps.icon.set` through the org's app configuration
// token: once when the app is created, and again whenever the avatar changes.
//
// Decision: best effort, off the request path. An icon is cosmetic, and Slack
// rate-limits `apps.icon.set` to about one call a minute, so a failure is
// logged rather than failing an upload or an install. Endpoints set up with
// the manual manifest flow have no configuration token; there the operator
// uploads the icon in Slack (the UI offers the 512px PNG for that).
// Removing an avatar leaves the last icon in place: Slack has no "reset".

use std::sync::Arc;

use uuid::Uuid;

use super::avatar::{AvatarShape, SLACK_ICON_SIZE, variant_name};
use crate::records::slack_provisioning::SlackAppProvisioner;
use crate::storage::{EncryptionService, StorageBackend};

/// The PNG to use as a Slack app icon for an avatar.
pub async fn slack_icon_png(db: &StorageBackend, avatar_id: Uuid) -> Option<Vec<u8>> {
    match db
        .get_agent_avatar_variant(
            avatar_id,
            &variant_name(AvatarShape::Square, SLACK_ICON_SIZE),
        )
        .await
    {
        Ok(row) => row.map(|row| row.data),
        Err(error) => {
            tracing::warn!(%avatar_id, %error, "Could not load avatar for Slack icon");
            None
        }
    }
}

/// Set one Slack app's icon from the agent's current avatar, if it has one.
pub async fn push_agent_avatar_to_slack_app(
    db: &StorageBackend,
    provisioner: &dyn SlackAppProvisioner,
    org_id: i64,
    agent_id: Uuid,
    team_id: Option<&str>,
    app_id: &str,
) {
    let avatar_id = match db
        .get_agent(
            org_id,
            everruns_contracts::typed_id::AgentId::from_uuid(agent_id),
        )
        .await
    {
        Ok(Some(row)) => row.avatar_id,
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(%agent_id, %error, "Could not load agent for Slack icon");
            None
        }
    };
    let Some(avatar_id) = avatar_id else {
        return;
    };
    let Some(png) = slack_icon_png(db, avatar_id).await else {
        return;
    };
    if let Err(error) = provisioner.set_app_icon(org_id, team_id, app_id, png).await {
        tracing::warn!(app_id, %error, "Could not set Slack app icon from agent avatar");
    }
}

/// Set the icon of every provisioned Slack app of an agent to `avatar_id`.
/// Returns how many apps were updated.
pub async fn push_avatar_to_agent_slack_apps(
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    provisioner: Arc<dyn SlackAppProvisioner>,
    org_id: i64,
    agent_id: Uuid,
    avatar_id: Uuid,
) -> usize {
    let channels = match db.list_agent_channels(org_id, agent_id).await {
        Ok(channels) => channels,
        Err(error) => {
            tracing::warn!(%agent_id, %error, "Could not list endpoints for Slack icon sync");
            return 0;
        }
    };
    let mut apps = Vec::new();
    for row in channels {
        if row.channel_type != "slack" {
            continue;
        }
        let Ok((_, channel)) =
            crate::domains::agent_channels::ingress::row_to_ingress(encryption.as_ref(), row)
        else {
            continue;
        };
        if let Some(app) = channel.slack_config().and_then(|c| c.provisioned_app) {
            apps.push(app);
        }
    }
    if apps.is_empty() {
        return 0;
    }
    let Some(png) = slack_icon_png(&db, avatar_id).await else {
        return 0;
    };
    let mut updated = 0;
    for app in apps {
        match provisioner
            .set_app_icon(org_id, app.team_id.as_deref(), &app.app_id, png.clone())
            .await
        {
            Ok(()) => updated += 1,
            Err(error) => {
                tracing::warn!(app_id = %app.app_id, %error, "Could not set Slack app icon from agent avatar");
            }
        }
    }
    updated
}
