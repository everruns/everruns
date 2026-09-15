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
