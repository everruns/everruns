// Browser access to `api` channels: the origins a channel lists in
// `cors_origins` may call that channel's own routes cross-origin.
//
// Decisions (see knowledge/integrations/agent-execution-api.md):
// - One CORS layer serves the whole app, so a channel's origins join the
//   server's global list inside it rather than in a second layer: the outer
//   layer answers every preflight and would hide an inner one.
// - A channel origin is allowed only for paths under that channel
//   (`/v1/channels/{id}` and below), never for the management API.
// - The global layer allows credentials. That grants a channel origin
//   nothing: channel routes read only a bearer token, never the session
//   cookie.
// THREAT[TM-AGENTKEY-008]: cross-origin reach of a channel's browser origins.

use std::sync::Arc;

use axum::http::HeaderValue;

use super::channel_ingress::resolve_channel;
use crate::domains::agent_channels::api_sessions::api_channel_config;
use crate::domains::agent_channels::record::ChannelType;
use crate::storage::{EncryptionService, StorageBackend};

const CHANNEL_PATH_PREFIX: &str = "/v1/channels/";

/// Looks up the browser origins of the `api` channel a request targets.
#[derive(Clone)]
pub struct ApiChannelOrigins {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
}

impl ApiChannelOrigins {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self { db, encryption }
    }

    /// Whether the `api` channel `path` targets lists `origin`.
    pub async fn allows(&self, path: &str, origin: &HeaderValue) -> bool {
        let (Some(channel_id), Ok(origin)) = (channel_id(path), origin.to_str()) else {
            return false;
        };
        let channel = match resolve_channel(&self.db, self.encryption.as_ref(), channel_id).await {
            Ok(Some((_, channel))) if channel.channel_type == ChannelType::Api => channel,
            Ok(_) => return false,
            Err(error) => {
                tracing::warn!(?error, "api channel CORS lookup failed");
                return false;
            }
        };
        api_channel_config(&channel)
            .is_ok_and(|config| config.cors_origins.iter().any(|allowed| allowed == origin))
    }
}

/// The channel id of a `/v1/channels/{id}[/...]` path.
fn channel_id(path: &str) -> Option<&str> {
    let id = path.strip_prefix(CHANNEL_PATH_PREFIX)?.split('/').next()?;
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::channel_id;

    #[test]
    fn only_channel_paths_name_a_channel() {
        assert_eq!(channel_id("/v1/channels/appchan_1"), Some("appchan_1"));
        assert_eq!(
            channel_id("/v1/channels/appchan_1/sessions/s/messages"),
            Some("appchan_1")
        );
        assert_eq!(channel_id("/v1/channels/"), None);
        assert_eq!(channel_id("/v1/agents/a/channels/appchan_1"), None);
        assert_eq!(channel_id("/v1/sessions"), None);
    }
}
