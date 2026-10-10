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
//!   [`ServerChannelResolver::kind`] does not name it.
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
        let kind = Self::kind(channel)?;
        let config = ChannelConfig {
            name: channel.internal_id.to_string(),
            agent: context.agent_id.map(|agent| agent.to_string()),
            binding: Self::binding(channel),
            delivery: Default::default(),
            scope: ChannelScope::new(ServerChannelScope {
                context: context.clone(),
                channel: channel.clone(),
            }),
        };
        Some(ResolvedChannel { config, kind })
    }

    /// How the host answers a channel type.
    fn kind(channel: &IngressChannel) -> Option<ChannelKind> {
        match channel.channel_type {
            ChannelType::Slack
            | ChannelType::Webhook
            | ChannelType::Api
            | ChannelType::ApiEndpoint
            | ChannelType::AgUi
            | ChannelType::PublicChat
            | ChannelType::Fcp
            | ChannelType::A2a
            | ChannelType::Schedule
            | ChannelType::Voice
            | ChannelType::Poppy => None,
        }
    }

    /// How a channel type's messages find their session.
    fn binding(_channel: &IngressChannel) -> SessionBinding {
        SessionBinding::Thread
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
