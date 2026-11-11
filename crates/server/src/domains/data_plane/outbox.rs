//! Delivers v2 orgs' config to their data planes (cp-dp-protocol.md,
//! "Config").
//!
//! Decisions:
//! - One dispatcher per server; an org is delivered by whichever server
//!   takes it, and a delivery is idempotent (versions apply once), so two
//!   servers racing on one org cost a duplicate request, never a wrong
//!   config.
//! - An org not yet enabled on its data plane is enabled first (`PUT
//!   /dp/v1/orgs/{org}` with its wrapped data key), then gets config.
//! - A new control plane epoch, or a data plane that answers
//!   `NEED_FULL_SNAPSHOT`, gets every live item with `base_version` 0. A
//!   data plane already at a newer epoch (`STALE_EPOCH`) is an operator's
//!   problem (a restore that was not followed by `dataplane bump-epoch`):
//!   the org backs off and the error is kept on its row.

use std::time::Duration;

use everruns_contracts::dataplane::{self as dp, ConfigAck, ConfigChange, ConfigUpdate};
use everruns_contracts::dataplane_seal as seal;
use serde_json::Value;

use super::{ControlPlane, DataPlane, org_public};
use crate::storage::{DP_CONFIG_CHANNEL, DpOrgRow};

/// The fallback poll when no notification arrives.
const POLL: Duration = Duration::from_secs(1);
/// Orgs delivered per pass.
const BATCH: i64 = 64;

/// Deliver due orgs until the process stops.
pub async fn run_dispatcher(cp: ControlPlane) {
    let mut listener = match sqlx::postgres::PgListener::connect_with(cp.db.background_pool()).await
    {
        Ok(mut listener) => match listener.listen(DP_CONFIG_CHANNEL).await {
            Ok(()) => Some(listener),
            Err(error) => {
                tracing::warn!(%error, "data plane config: no LISTEN, polling only");
                None
            }
        },
        Err(error) => {
            tracing::warn!(%error, "data plane config: no LISTEN, polling only");
            None
        }
    };
    loop {
        if let Err(error) = deliver_due(&cp).await {
            tracing::warn!(%error, "data plane config not delivered");
        }
        match listener.as_mut() {
            Some(listener) => {
                let _ = tokio::time::timeout(POLL, listener.recv()).await;
            }
            None => tokio::time::sleep(POLL).await,
        }
    }
}

/// One pass: every org that is behind and due.
pub async fn deliver_due(cp: &ControlPlane) -> anyhow::Result<usize> {
    let epoch = cp.db.dp_epoch().await?;
    let due = cp.db.dp_orgs_due(epoch, BATCH).await?;
    let n = due.len();
    let mut tasks = tokio::task::JoinSet::new();
    for org in due {
        let cp = cp.clone();
        tasks.spawn(async move {
            if let Err(error) = deliver(&cp, &org, epoch).await {
                tracing::debug!(org_id = org.org_id, %error, "data plane config delivery failed");
                let _ = cp
                    .db
                    .dp_delivery_failed(org.org_id, &format!("{error:#}"))
                    .await;
            }
        });
    }
    while tasks.join_next().await.is_some() {}
    Ok(n)
}

/// Bring `org`'s data plane up to its current config.
pub async fn deliver(cp: &ControlPlane, org: &DpOrgRow, epoch: i64) -> anyhow::Result<()> {
    let plane = super::for_org(cp, org.org_id).await?;
    let public = org_public(org.org_id);
    if !org.enabled {
        enable(cp, plane.as_ref(), org, &public).await?;
        cp.db.dp_set_org_enabled(org.org_id).await?;
    }
    let full = org.delivered_epoch != epoch;
    let update = build(cp, org, epoch, full).await?;
    let mut ack = plane.push_config(&public, &update).await?;
    if matches!(ack, ConfigAck::NeedFullSnapshot { .. }) && !full {
        let update = build(cp, org, epoch, true).await?;
        ack = plane.push_config(&public, &update).await?;
    }
    match ack {
        ConfigAck::Ok { applied_version } | ConfigAck::AlreadyApplied { applied_version } => {
            cp.db
                .dp_delivered(org.org_id, epoch, applied_version as i64)
                .await
        }
        ConfigAck::NeedFullSnapshot { applied_version } => anyhow::bail!(
            "the data plane wants a full snapshot again (it applied {applied_version})"
        ),
        ConfigAck::StaleEpoch { epoch: theirs } => anyhow::bail!(
            "the data plane is at epoch {theirs}, this control plane at {epoch}: bump the epoch after resolving the config diff"
        ),
    }
}

async fn enable(
    cp: &ControlPlane,
    plane: &dyn DataPlane,
    org: &DpOrgRow,
    public: &str,
) -> anyhow::Result<()> {
    let dp_key = cp
        .link
        .dp_key
        .ok_or_else(|| anyhow::anyhow!("EVERRUNS_DP_KEY is not set"))?;
    let data_key = cp.data_key(org)?;
    let wrapped = seal::wrap_data_key(
        &dp_key,
        public,
        &org.data_key_id,
        &data_key,
        seal::random_nonce(),
    )?;
    let enable = dp::OrgEnable {
        // One command per data key: a retry is the same command.
        command_id: format!("enable-{}", org.data_key_id),
        wrapped_data_key: wrapped,
        data_key_id: org.data_key_id.clone(),
        policy: Default::default(),
    };
    let ack: dp::CommandAck = serde_json::from_value(
        plane
            .admin(
                reqwest::Method::PUT,
                &format!("{}/orgs/{public}", dp::PREFIX),
                &serde_json::to_value(&enable)?,
            )
            .await?,
    )?;
    anyhow::ensure!(
        ack.accepted,
        "the data plane refused to enable the org: {}",
        ack.error.unwrap_or_default()
    );
    Ok(())
}

/// The update that brings the data plane from what it applied to now.
async fn build(
    cp: &ControlPlane,
    org: &DpOrgRow,
    epoch: i64,
    full: bool,
) -> anyhow::Result<ConfigUpdate> {
    let (items, base) = if full {
        (cp.db.dp_items_live(org.org_id).await?, 0)
    } else {
        (
            cp.db
                .dp_items_since(org.org_id, org.delivered_version)
                .await?,
            org.delivered_version,
        )
    };
    let version = items
        .iter()
        .map(|item| item.version)
        .max()
        .unwrap_or(org.version)
        .max(org.version);
    let changes: Vec<Value> = items
        .into_iter()
        .map(|item| match item.body {
            Some(body) => body,
            None => ConfigChange::Removal {
                of: item.kind,
                id: item.item_id,
            }
            .to_value(),
        })
        .collect();
    Ok(ConfigUpdate {
        epoch: epoch as u64,
        version: version as u64,
        base_version: base as u64,
        changes,
    })
}
