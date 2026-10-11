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
//! - `api` and `api_endpoint` are explicit session APIs and stay off the
//!   host; schedules, voice and Poppy reach sessions through the port only.
//! - Slack channels run only on a resolver given the Slack state
//!   ([`ServerChannelResolver::with_slack`]): its driver posts with it.
//!
//! [`resolve`]: ChannelResolver::resolve

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::runtime::channel::{ChannelError, SessionBinding};
use everruns_core::channel_runtime::{
    ChannelConfig, ChannelResolver, ChannelScope, ResolvedChannel,
};
use uuid::Uuid;

use std::collections::HashMap;

use everruns_core::channel_runtime::DeliveryOptions;
use serde_json::Value;

use super::ServerChannelScope;
use super::webhook::WebhookDriver;
use crate::channels::slack::approvals::SLACK_APPROVAL_HINT;
use crate::channels::slack::events::SlackState;
use crate::channels::slack::host::{SLACK_THREAD_KEY, SlackHostDriver, SlackTurnPlanner};
use crate::domains::agent_channels::ingress::{
    IngressChannel, IngressContext, channel_liveness, row_to_ingress,
};
use crate::domains::agent_channels::record::ChannelType;
use crate::domains::agent_channels::record::slack_channel::SlackChannelConfig;
use crate::storage::{EncryptionService, StorageBackend};

/// [`ChannelResolver`] over `agent_channels`.
#[derive(Clone)]
pub struct ServerChannelResolver {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    /// What Slack channels run with. A resolver without it does not run them.
    slack: Option<SlackState>,
}

impl ServerChannelResolver {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self {
            db,
            encryption,
            slack: None,
        }
    }

    /// Run Slack channels with `state`.
    pub fn with_slack(mut self, state: SlackState) -> Self {
        self.slack = Some(state);
        self
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
    pub fn channel_for(&self, mut scope: ServerChannelScope) -> Option<ResolvedChannel> {
        let mut binding = SessionBinding::Thread;
        let mut delivery = DeliveryOptions::default();
        let mut slack = None;
        let kind = match scope.channel.channel_type {
            ChannelType::Webhook => {
                let webhook = scope.channel.webhook_config()?;
                binding = webhook.session_mode;
                delivery.replies = false;
                Kind::Webhook(webhook.message)
            }
            // The kind is the routing namespace the channel's sessions had.
            ChannelType::AgUi => Kind::Streaming("ag_ui"),
            ChannelType::PublicChat => Kind::Streaming("public_chat"),
            ChannelType::Fcp => Kind::Streaming("fcp"),
            ChannelType::A2a => {
                binding = scope.channel.a2a_config()?.session_mode;
                Kind::Streaming("a2a")
            }
            ChannelType::Slack => {
                let state = self.slack.clone()?;
                let config: SlackChannelConfig =
                    serde_json::from_value(scope.channel.channel_config.clone()).ok()?;
                binding = config.session_strategy;
                // EVE-1025: a Slack thread can draw an approval card, so its
                // sessions say so.
                scope.session_hints = Some(HashMap::from([(
                    SLACK_APPROVAL_HINT.to_string(),
                    Value::Bool(true),
                )]));
                scope.participant_thread_key = Some(SLACK_THREAD_KEY);
                slack = Some((state, config));
                Kind::Slack
            }
            ChannelType::Api | ChannelType::Schedule | ChannelType::Voice | ChannelType::Poppy => {
                return None;
            }
        };
        let config = ChannelConfig {
            name: scope.channel.internal_id.to_string(),
            agent: scope.context.agent_id.map(|agent| agent.to_string()),
            binding,
            delivery,
            scope: ChannelScope::new(scope.clone()),
        };
        Some(match kind {
            Kind::Webhook(message) => {
                ResolvedChannel::messaging(config, Arc::new(WebhookDriver::new(scope, message)))
            }
            Kind::Streaming(kind) => ResolvedChannel::streaming(config, kind),
            Kind::Slack => {
                let (state, slack) = slack?;
                let ServerChannelScope {
                    context, channel, ..
                } = scope;
                let planner = SlackTurnPlanner::new(state.clone(), context.org_id, slack.clone());
                let driver = SlackHostDriver::new(state, context, channel, slack);
                ResolvedChannel::messaging(config, Arc::new(driver)).with_planner(Arc::new(planner))
            }
        })
    }
}

enum Kind {
    Webhook(String),
    Streaming(&'static str),
    Slack,
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
