use std::sync::Arc;

use everruns_platform::{App, AppChannel, ChannelType};

use crate::domains::apps::queries;
use crate::storage::{EncryptionService, StorageBackend};

pub async fn resolve_endpoint(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    channel_id: &str,
) -> anyhow::Result<Option<(App, AppChannel)>> {
    let Some(app) = queries::get_by_channel_public_id_unscoped(db, encryption, channel_id).await?
    else {
        return Ok(None);
    };
    let channel = app
        .channels
        .iter()
        .find(|channel| channel.public_id.to_string() == channel_id)
        .cloned();
    Ok(channel.map(|channel| (app, channel)))
}

/// Why an endpoint is not accepting traffic. Callers collapse every variant into
/// one generic rejection; this exists so the reason can be logged server-side.
///
/// THREAT[TM-TENANT-002]: an unauthenticated caller must not be able to tell
/// "endpoint does not exist" from "endpoint exists but is not live". Do not
/// surface these variants, or distinct status codes for them, to a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotLive {
    /// The endpoint's own status is `draft` or `disabled`.
    EndpointNotLive,
    /// The owning agent is archived or deleted.
    AgentNotActive,
    /// The agent-level incident switch is on.
    ExposuresSuspended,
    /// The endpoint has no owning agent to resolve liveness against.
    NoAgent,
}

impl NotLive {
    pub fn as_str(self) -> &'static str {
        match self {
            NotLive::EndpointNotLive => "endpoint not live",
            NotLive::AgentNotActive => "agent not active",
            NotLive::ExposuresSuspended => "agent exposures suspended",
            NotLive::NoAgent => "endpoint has no agent",
        }
    }
}

/// Decide whether an endpoint accepts traffic (EVE-1007):
///
/// ```text
/// live(endpoint) = endpoint.status == live
///               && agent.status == active
///               && !agent.exposures_suspended
/// ```
///
/// The agent-level terms are folded in **here, at resolution time**, and are
/// never denormalized onto the endpoint row. Writing rows when an agent is
/// archived would create a second writer for endpoint status and make the
/// archive un-restorable — the same reasoning that keeps the harness overlay
/// chain behind the platform loading boundary.
///
/// `App.status` and `AppChannel.enabled` are deliberately **not** consulted.
/// They were the old two-dimensional gate, and reading them here would put back
/// the coupling that made publishing one endpoint expose its siblings.
pub async fn endpoint_liveness(
    db: &StorageBackend,
    app: &App,
    endpoint: &AppChannel,
) -> anyhow::Result<Result<(), NotLive>> {
    if !endpoint.status.is_live() {
        return Ok(Err(NotLive::EndpointNotLive));
    }

    let Some(agent_id) = app.agent_id.as_ref() else {
        return Ok(Err(NotLive::NoAgent));
    };
    // `App::agent_id` carries the agent's *public* id, so resolve by that rather
    // than treating it as an internal key — the two coincide only when the
    // public id happened to be minted from the internal uuid.
    let Some(agent) = db
        .get_agent_by_public_id(app.org_id, &agent_id.to_string())
        .await?
    else {
        return Ok(Err(NotLive::NoAgent));
    };

    if agent.status != "active" {
        return Ok(Err(NotLive::AgentNotActive));
    }
    if agent.exposures_suspended {
        return Ok(Err(NotLive::ExposuresSuspended));
    }
    Ok(Ok(()))
}

pub enum LegacyChannelMatch {
    NotFound,
    One(AppChannel),
    Ambiguous,
}

pub fn resolve_legacy_channel(app: &App, channel_type: ChannelType) -> LegacyChannelMatch {
    let mut channels = app
        .channels
        .iter()
        .filter(|channel| channel.channel_type == channel_type && channel.enabled);
    let Some(channel) = channels.next().cloned() else {
        return LegacyChannelMatch::NotFound;
    };
    if channels.next().is_some() {
        LegacyChannelMatch::Ambiguous
    } else {
        LegacyChannelMatch::One(channel)
    }
}
