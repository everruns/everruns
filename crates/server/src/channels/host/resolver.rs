//! Channel rows as the host's channels.
//!
//! Decisions:
//! - The host name is the row's internal id ([`super`]); [`resolve`] is what
//!   recovery calls, [`ServerChannelResolver::channel`] is what an HTTP
//!   handler calls once it has resolved and authorized a row by public id.
//! - A row that is gone, not live, or of a type the host does not run yet
//!   resolves to `None`: its pending deliveries are dropped, as replies to a
//!   disabled channel should be. A read that fails is an error, which keeps
//!   them for a later recovery.
//! - Each channel type is moved onto the host in its own change; until then
//!   [`ServerChannelResolver::channel`] does not run it.
//!
//! [`resolve`]: ChannelResolver::resolve

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::runtime::channel::{ChannelError, SessionBinding};
use everruns_core::channel_runtime::{
    ChannelConfig, ChannelKind, ChannelResolver, ChannelScope, ResolvedChannel,
};
use uuid::Uuid;

use super::ServerChannelScope;
use super::webhook::WebhookDriver;
use crate::domains::agent_channels::ingress::{
    IngressChannel, IngressContext, channel_liveness, row_to_ingress,
};
use crate::domains::agent_channels::record::ChannelType;
use crate::storage::{EncryptionService, StorageBackend};

/// [`ChannelResolver`] over `agent_channels`.
#[derive(Clone)]
pub struct ServerChannelResolver {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
}

impl ServerChannelResolver {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self { db, encryption }
    }

    /// The host channel for a resolved row, or `None` when the host does not
    /// run this channel type.
    pub fn channel(
        &self,
        context: &IngressContext,
        channel: &IngressChannel,
    ) -> Option<ResolvedChannel> {
        self.channel_for(ServerChannelScope::new(context.clone(), channel.clone()))
    }

    /// [`channel`](Self::channel) for a request that adds to the scope: the
    /// end user it runs as, its request id.
    pub fn channel_for(&self, scope: ServerChannelScope) -> Option<ResolvedChannel> {
        let ServerChannelScope {
            context, channel, ..
        } = &scope;
        let mut config = ChannelConfig {
            name: channel.internal_id.to_string(),
            agent: context.agent_id.map(|agent| agent.to_string()),
            binding: SessionBinding::Thread,
            delivery: Default::default(),
            scope: ChannelScope::new(scope.clone()),
        };
        let kind = match channel.channel_type {
            ChannelType::Webhook => {
                let webhook = channel.webhook_config()?;
                config.binding = webhook.session_mode;
                config.delivery.replies = false;
                ChannelKind::Messaging(Arc::new(WebhookDriver::new(scope, webhook.message)))
            }
            // The kind is the routing namespace the channel's sessions had.
            ChannelType::AgUi => ChannelKind::Streaming("ag_ui".into()),
            ChannelType::PublicChat => ChannelKind::Streaming("public_chat".into()),
            ChannelType::Fcp => ChannelKind::Streaming("fcp".into()),
            ChannelType::A2a => {
                config.binding = channel.a2a_config()?.session_mode;
                ChannelKind::Streaming("a2a".into())
            }
            ChannelType::Slack
            | ChannelType::Api
            | ChannelType::ApiEndpoint
            | ChannelType::Schedule
            | ChannelType::Voice
            | ChannelType::Poppy => return None,
        };
        Some(ResolvedChannel { config, kind })
    }
}

#[async_trait]
impl ChannelResolver for ServerChannelResolver {
    async fn resolve(&self, name: &str) -> Result<Option<ResolvedChannel>, ChannelError> {
        let Ok(channel_id) = Uuid::parse_str(name) else {
            return Ok(None);
        };
        let Some(row) = self.db.get_ingress_channel_by_id(channel_id).await? else {
            return Ok(None);
        };
        let (context, channel) = row_to_ingress(self.encryption.as_ref(), row)?;
        if channel_liveness(&context, &channel).is_err() {
            return Ok(None);
        }
        Ok(self.channel(&context, &channel))
    }
}
