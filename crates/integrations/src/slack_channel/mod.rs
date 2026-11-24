//! A Slack channel driver: Events API in, `chat.postMessage` out.
//!
//! In: `app_mention` events, and plain messages unless the channel is
//! mention-only. Each Slack thread is one conversation; a message outside a
//! thread starts one. Out: replies are posted in that thread.
//!
//! Decisions:
//! - Requests are verified with Slack's `v0` signature and refused when the
//!   timestamp is more than five minutes off, so a captured request cannot be
//!   replayed.
//! - Signed HTTP always requires `SLACK_SIGNING_SECRET`, including local
//!   `from_env` drivers. Channelkit never treats HTTP as trusted Socket Mode.
//! - Without a bot token, replies are logged instead of posted.
//! - The binding thread is `channel:thread_ts`: Slack timestamps are unique
//!   per channel only.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use everruns_contracts::runtime::channel::{
    ChannelDeliveryAdapter, ChannelDriver, ChannelError, ChannelRequest, ChannelResponse,
    DeliveryContext, DeliveryResult, DeliveryTarget, ExternalActor, Inbound, InboundAttachment,
    InboundChannelEvent, InboundMessage, OutboundChannelMessage,
};
use serde_json::{Value, json};
use tracing::info;

use bridge::{INSTALLATION_ID, SlackAdapterFactory, TEAM_ID};
use std::sync::Arc;
use web_api::SLACK_API_BASE;

pub mod actions;
mod advanced;
pub mod bridge;
pub mod channelkit_surfaces;
pub mod ingress;
pub mod markdown;
pub mod surfaces;
pub mod web_api;
pub use advanced::{Participation, SlackAdmission};

const BOT_TOKEN_ENV: &str = "SLACK_BOT_TOKEN";
const SIGNING_SECRET_ENV: &str = "SLACK_SIGNING_SECRET";

#[derive(Clone)]
enum Credential {
    /// Read from this environment variable at use, so a host can set it
    /// after the channel is built.
    Env(&'static str),
    Value(String),
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}

impl Credential {
    fn value(&self) -> Option<String> {
        match self {
            Self::Env(name) => std::env::var(name).ok().filter(|value| !value.is_empty()),
            Self::Value(value) => (!value.is_empty()).then(|| value.clone()),
        }
    }
}

/// The Slack driver.
#[derive(Clone)]
pub struct Slack {
    token: Credential,
    signing_secret: Credential,
    mention_only: bool,
    api_base: String,
    advanced: advanced::Advanced,
    workspace: Option<String>,
    installation: String,
    factory: Option<Arc<SlackAdapterFactory>>,
}

impl Slack {
    /// A driver with a bot token (`xoxb-…`) and the app's signing secret.
    pub fn new(bot_token: impl Into<String>, signing_secret: impl Into<String>) -> Self {
        Self::with(
            Credential::Value(bot_token.into()),
            Credential::Value(signing_secret.into()),
        )
    }

    /// A driver reading `SLACK_BOT_TOKEN` and `SLACK_SIGNING_SECRET` from the
    /// environment when it needs them.
    pub fn from_env() -> Self {
        Self::with(
            Credential::Env(BOT_TOKEN_ENV),
            Credential::Env(SIGNING_SECRET_ENV),
        )
    }

    fn with(token: Credential, signing_secret: Credential) -> Self {
        Self {
            token,
            signing_secret,
            mention_only: false,
            api_base: SLACK_API_BASE.to_string(),
            advanced: advanced::Advanced::default(),
            workspace: None,
            installation: "everruns-sdk-slack".into(),
            factory: None,
        }
    }

    /// Accept events from this Slack workspace only. Recommended for apps installed in multiple workspaces.
    pub fn workspace(mut self, team_id: impl Into<String>) -> Self {
        self.workspace = Some(team_id.into());
        self
    }
    /// Answer only when the bot is @-mentioned.
    pub fn mention_only(mut self) -> Self {
        self.mention_only = true;
        self
    }

    /// Post to another Web API base, such as a test server.
    pub fn api_base(mut self, url: impl Into<String>) -> Self {
        self.api_base = url.into();
        self
    }

    /// Stable host-owned installation identity; use a different name for each app.
    pub fn installation(mut self, id: impl Into<String>) -> Self {
        self.installation = id.into();
        self
    }

    /// Use this service for provider requests outside a host-supplied context.
    /// Host delivery and file intake always retain their configured scoped egress.
    pub fn egress(
        self,
        egress: Arc<dyn everruns_contracts::runtime::egress::EgressService>,
    ) -> Self {
        self.transport(Arc::new(crate::channelkit_bridge::EgressTransport::new(
            egress,
        )))
    }

    /// Inject a Channelkit transport, shared by every provider operation.
    pub fn transport(mut self, transport: channelkit::http::SharedTransport) -> Self {
        self.factory = Some(Arc::new(SlackAdapterFactory::new(transport)));
        self
    }

    fn factory(&self) -> channelkit::Result<&SlackAdapterFactory> {
        match &self.factory {
            Some(factory) => Ok(factory),
            None => bridge::direct_factory(),
        }
    }

    fn scoped_context(&self, context: &DeliveryContext) -> Result<DeliveryContext, DeliveryResult> {
        if context
            .extra
            .get(INSTALLATION_ID)
            .is_some_and(|id| id != &self.installation)
        {
            return Err(bridge::outcome(&channelkit::Error::Scope));
        }
        if let Some(team) = &self.workspace {
            for key in [TEAM_ID, surfaces::SLACK_RECIPIENT_TEAM_ID] {
                if context
                    .extra
                    .get(key)
                    .filter(|value| !value.is_empty())
                    .is_some_and(|value| value != team)
                {
                    return Err(bridge::outcome(&channelkit::Error::Scope));
                }
            }
        }
        let mut scoped = context.clone();
        scoped
            .extra
            .insert(INSTALLATION_ID.into(), self.installation.clone());
        if let Some(team) = &self.workspace {
            scoped
                .extra
                .entry(TEAM_ID.into())
                .or_insert_with(|| team.clone());
        }
        Ok(scoped)
    }

    fn provider_surfaces(&self) -> channelkit::Result<channelkit_surfaces::ChannelkitSurfaces<'_>> {
        Ok(channelkit_surfaces::ChannelkitSurfaces {
            api_base: &self.api_base,
            factory: self.factory()?,
        })
    }
}

#[async_trait]
impl ChannelDeliveryAdapter for Slack {
    fn platform(&self) -> &str {
        "slack"
    }

    fn requires_checkpoint(&self) -> bool {
        true
    }

    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        let context = match self.scoped_context(context) {
            Ok(context) => context,
            Err(error) => return error,
        };
        let channel = &context.channel_id;
        if context.auth_token.is_empty() {
            info!(%channel, thread = %message.thread_ref, text = %message.text, "slack reply (no {BOT_TOKEN_ENV}, not posted)");
            return DeliveryResult::Ok;
        }
        let surfaces = match self.provider_surfaces() {
            Ok(surfaces) => surfaces,
            Err(error) => return bridge::outcome(&error),
        };
        let session = message.session_id.to_string();
        surfaces
            .post_reply(
                &message.text,
                &context,
                message
                    .correlation_id
                    .as_deref()
                    .map(|input| (session.as_str(), input)),
            )
            .await
            .map_or_else(|error| error, |_| DeliveryResult::Ok)
    }

    fn streaming(
        &self,
    ) -> Option<&dyn everruns_contracts::runtime::channel::ChannelStreamDelivery> {
        self.advanced.pane.then_some(self)
    }
    fn agent_surface(
        &self,
    ) -> Option<&dyn everruns_contracts::runtime::channel::ChannelAgentSurface> {
        self.advanced.pane.then_some(self)
    }
    fn progress(
        &self,
    ) -> Option<&dyn everruns_contracts::runtime::channel::ChannelProgressSurface> {
        self.advanced.progress.then_some(self)
    }
    fn approvals(
        &self,
    ) -> Option<&dyn everruns_contracts::runtime::channel::ChannelApprovalPrompt> {
        self.advanced.approvals.then_some(self)
    }
    async fn send_ack(&self, _: &str, _: &str, _: &DeliveryContext) -> DeliveryResult {
        DeliveryResult::Ok
    }
}

#[async_trait]
impl ChannelDriver for Slack {
    fn secrets(&self) -> Vec<String> {
        [&self.token, &self.signing_secret]
            .into_iter()
            .filter_map(|credential| match credential {
                Credential::Env(name) => Some(name.to_string()),
                Credential::Value(_) => None,
            })
            .collect()
    }

    async fn receive(&self, request: &ChannelRequest) -> Result<Inbound, ChannelError> {
        self.receive_with_egress(request, None).await
    }
    async fn receive_with_egress(
        &self,
        request: &ChannelRequest,
        egress: Option<Arc<dyn everruns_contracts::runtime::egress::EgressService>>,
    ) -> Result<Inbound, ChannelError> {
        let secret = self
            .signing_secret
            .value()
            .ok_or_else(|| ChannelError::Unauthorized("Slack signing secret is required".into()))?;
        let token = self.token.value();
        let ingress = ingress::HttpIngress {
            installation: &self.installation,
            team: self.workspace.as_deref(),
            token: token.as_deref(),
            signing_secret: &secret,
            api: &self.api_base,
            factory: self.factory().map_err(ingress::ingress_error)?,
        }
        .receive(request, egress)
        .await?;
        match ingress {
            ingress::VerifiedHttpIngress::Challenge(challenge) => Ok(Inbound::Respond(
                ChannelResponse::ok(json!({"challenge":challenge})),
            )),
            ingress::VerifiedHttpIngress::Events { team, ingress } => {
                let Some(event) = ingress.ingress.events.into_iter().next() else {
                    return Ok(Inbound::Ignore);
                };
                self.normalized(event.kind, &ingress.context, &team)
            }
        }
    }

    fn attachment_resolver(
        &self,
    ) -> Option<&dyn everruns_contracts::runtime::channel::ChannelAttachmentResolver> {
        Some(self)
    }

    fn binding(
        &self,
        _route: &everruns_contracts::runtime::channel::InboundRoute,
        _configured: everruns_contracts::runtime::channel::SessionBinding,
    ) -> everruns_contracts::runtime::channel::SessionBinding {
        everruns_contracts::runtime::channel::SessionBinding::Thread
    }
    async fn admit(
        &self,
        message: &InboundMessage,
        session_id: Option<&str>,
    ) -> Result<bool, ChannelError> {
        match &self.advanced.admission {
            Some(policy) => policy.admit(message, session_id).await,
            None => Ok(true),
        }
    }
    async fn backfill(
        &self,
        message: &InboundMessage,
    ) -> Result<Vec<InboundChannelEvent>, ChannelError> {
        self.history(message, &self.delivery_context(&message.reply_to))
            .await
    }
    async fn backfill_with_context(
        &self,
        message: &InboundMessage,
        context: &DeliveryContext,
    ) -> Result<Vec<InboundChannelEvent>, ChannelError> {
        self.history(message, context).await
    }
    fn delivery_context(&self, target: &DeliveryTarget) -> DeliveryContext {
        let mut context = target.context(self.token.value().unwrap_or_default());
        context
            .extra
            .entry(INSTALLATION_ID.into())
            .or_insert_with(|| self.installation.clone());
        if let Some(team) = &self.workspace {
            // Keep a persisted original workspace for mismatch detection.
            context
                .extra
                .entry(TEAM_ID.into())
                .or_insert_with(|| team.clone());
        }
        context
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// Remove `<@U123>` mention tokens and collapse the whitespace around them.
fn strip_mentions(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => rest = &rest[start + end + 1..],
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl std::fmt::Debug for Slack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slack")
            .field("installation", &self.installation)
            .field("workspace", &self.workspace)
            .field("mention_only", &self.mention_only)
            .field("advanced", &self.advanced)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl everruns_contracts::runtime::channel::ChannelAttachmentResolver for Slack {
    async fn download(
        &self,
        reference: &str,
        max_bytes: usize,
        egress: Arc<dyn everruns_contracts::runtime::egress::EgressService>,
    ) -> Result<Vec<u8>, ChannelError> {
        let (installation, team, id): (String, String, String) = serde_json::from_str(reference)
            .map_err(|_| ChannelError::BadRequest("Slack file reference".into()))?;
        if installation != self.installation
            || id.is_empty()
            || id.len() > 256
            || self
                .workspace
                .as_ref()
                .is_some_and(|expected| expected != &team)
        {
            return Err(ChannelError::Unauthorized("Slack file scope".into()));
        }
        let token = self.token.value().ok_or_else(|| {
            ChannelError::Unauthorized("Slack file credentials unavailable".into())
        })?;
        let factory = self.factory().map_err(ingress::ingress_error)?;
        let client = factory
            .for_credentials(
                &self.installation,
                Some(&team),
                &token,
                &self.api_base,
                Some(egress),
            )
            .await
            .map_err(ingress::ingress_error)?;
        client
            .download(
                &channelkit::Attachment {
                    id,
                    name: None,
                    media_type: None,
                    size: None,
                },
                max_bytes,
            )
            .await
            .map_err(|_| ChannelError::BadRequest("Slack file unavailable".into()))
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod migration_tests;
