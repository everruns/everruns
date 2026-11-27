//! Channelkit's authenticated ingress mapped onto the existing SDK binding keys.
use super::*;
use channelkit::{Control, EventKind};
use channelkit_slack::SlackEventContext;

/// Configuration already selected by the host; never selected by webhook fields.
/// No Debug: the credentials are used only for verification and scoped discovery.
pub struct HttpIngress<'a> {
    pub installation: &'a str,
    pub team: Option<&'a str>,
    pub token: Option<&'a str>,
    pub signing_secret: &'a str,
    pub api: &'a str,
    pub factory: &'a SlackAdapterFactory,
}

pub enum VerifiedHttpIngress {
    Challenge(String),
    Events {
        team: String,
        ingress: Box<channelkit_slack::SlackIngress>,
    },
}

impl HttpIngress<'_> {
    /// Verify original bytes before reading routing scope or making provider calls.
    pub async fn receive(
        &self,
        request: &ChannelRequest,
        egress: Option<Arc<dyn everruns_contracts::runtime::egress::EgressService>>,
    ) -> Result<VerifiedHttpIngress, ChannelError> {
        if self.signing_secret.trim().is_empty() {
            return Err(ChannelError::Unauthorized(
                "Slack signing secret unavailable".into(),
            ));
        }
        if request.body.len() > channelkit::MAX_WEBHOOK_BYTES {
            return Err(ChannelError::BadRequest(
                "Slack request exceeds limit".into(),
            ));
        }
        let raw = channelkit::WebhookRequest {
            method: "POST".into(),
            query: Default::default(),
            headers: request.headers.iter().cloned().collect(),
            body: request.body.clone(),
        };
        channelkit_slack::verify_signature(self.signing_secret, &raw, unix_now().max(0) as u64)
            .map_err(ingress_error)?;
        let value: Value = if request
            .header_value("content-type")
            .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"))
        {
            let payload = url::form_urlencoded::parse(&request.body)
                .find(|(key, _)| key == "payload")
                .map(|(_, v)| v.into_owned())
                .ok_or_else(|| ChannelError::BadRequest("Slack interaction payload".into()))?;
            serde_json::from_str(&payload)
                .map_err(|_| ChannelError::BadRequest("Slack interaction JSON".into()))?
        } else {
            request.body_json()?
        };
        if value["type"] == "url_verification" {
            let challenge = value["challenge"]
                .as_str()
                .filter(|value| !value.is_empty() && value.len() <= 4096)
                .ok_or_else(|| ChannelError::BadRequest("Slack challenge".into()))?;
            return Ok(VerifiedHttpIngress::Challenge(challenge.into()));
        }
        let team = value["team_id"]
            .as_str()
            .or_else(|| value["team"]["id"].as_str())
            .ok_or_else(|| ChannelError::BadRequest("Slack workspace is required".into()))?;
        if let Some(expected) = self.team {
            if expected != team {
                return Err(ChannelError::Unauthorized(
                    "Slack workspace mismatch".into(),
                ));
            }
        } else {
            // An app's signing secret is shared across workspace installations.
            // A signed team alone cannot select one host's session namespace.
            let token = self.token.ok_or_else(|| {
                ChannelError::Unauthorized(
                    "Configure a Slack workspace when no bot token is available".into(),
                )
            })?;
            tokio::time::timeout(
                std::time::Duration::from_millis(1500),
                self.factory.for_credentials(
                    self.installation,
                    Some(team),
                    token,
                    self.api,
                    egress,
                ),
            )
            .await
            .map_err(|_| ChannelError::Session("Slack installation discovery timed out".into()))?
            .map_err(|error| match error {
                channelkit::Error::Scope | channelkit::Error::Authentication => {
                    ChannelError::Unauthorized("Slack installation scope mismatch".into())
                }
                _ => ChannelError::Session("Slack installation discovery unavailable".into()),
            })?;
        }
        let client = self
            .factory
            .get(
                self.installation,
                team,
                None,
                Some(self.signing_secret),
                self.api,
            )
            .map_err(ingress_error)?;
        let ingress = client.receive_with_context(&raw).map_err(ingress_error)?;
        Ok(VerifiedHttpIngress::Events {
            team: team.into(),
            ingress: Box::new(ingress),
        })
    }
}

impl Slack {
    pub(super) fn normalized(
        &self,
        kind: EventKind,
        context: &SlackEventContext,
        team: &str,
    ) -> Result<Inbound, ChannelError> {
        match kind {
            EventKind::Message(message) => {
                if self.mention_only && !message.mentioned {
                    return Ok(Inbound::Ignore);
                }
                // Preserve the SDK's existing subtype policy. Hosted Slack may
                // admit file_share/thread_broadcast independently.
                if context.subtype.is_some() {
                    return Ok(Inbound::Ignore);
                }
                let channel = &message.reference.conversation.channel;
                let ts = &message.reference.id;
                let thread = message
                    .reference
                    .conversation
                    .thread
                    .as_deref()
                    .unwrap_or(ts);
                let user = &message.author.id;
                self.clear_approval_cards(channel, thread);
                Ok(Inbound::Message(Box::new(InboundMessage {
                    event: InboundChannelEvent {
                        actor: ExternalActor {
                            actor_id: user.clone(),
                            actor_name: message.author.display_name,
                            source: "slack".into(),
                            metadata: None,
                        },
                        text: strip_mentions(&message.text),
                        attachments: provider_attachments(
                            context,
                            &self.installation,
                            team,
                            self.token.value().is_some(),
                        ),
                        dedup_key: format!("{channel}:{ts}"),
                        thread_ref: Some(format!("{channel}:{thread}")),
                        routing_metadata: HashMap::from([
                            ("channel_id".into(), channel.clone()),
                            ("user_id".into(), user.clone()),
                        ]),
                        message_metadata: serde_json::Map::from_iter([
                            ("slack_channel".into(), json!(channel)),
                            ("slack_thread_ts".into(), json!(thread)),
                            ("slack_ts".into(), json!(ts)),
                            ("slack_user".into(), json!(user)),
                            ("slack_mention".into(), json!(message.mentioned)),
                        ]),
                    },
                    reply_to: {
                        let mut target = advanced::target(channel, thread, user, team);
                        target.extra.insert(bridge::TEAM_ID.into(), team.into());
                        target
                            .extra
                            .insert(bridge::INSTALLATION_ID.into(), self.installation.clone());
                        target
                    },
                })))
            }
            EventKind::Action(action) => self.interactive_action(&action),
            EventKind::Control {
                conversation,
                actor,
                action,
            } if self.advanced.pane => {
                let action = match action {
                    Control::Stop if actor.is_some() => {
                        everruns_contracts::runtime::channel::ControlAction::Cancel
                    }
                    Control::Title(title) => {
                        everruns_contracts::runtime::channel::ControlAction::Rename {
                            title: title.chars().take(250).collect(),
                        }
                    }
                    Control::View(value) => {
                        everruns_contracts::runtime::channel::ControlAction::Context {
                            key: "channel_view_context".into(),
                            value: serde_json::to_value(value).unwrap_or_default(),
                        }
                    }
                    _ => return Ok(Inbound::Ignore),
                };
                let user = actor.map(|a| a.id).unwrap_or_default();
                let thread = conversation.thread.unwrap_or_default();
                Ok(Inbound::Control(Box::new(
                    everruns_contracts::runtime::channel::InboundControl {
                        route: everruns_contracts::runtime::channel::InboundRoute {
                            actor: ExternalActor {
                                actor_id: user.clone(),
                                actor_name: None,
                                source: "slack".into(),
                                metadata: None,
                            },
                            thread_ref: Some(format!("{}:{thread}", conversation.channel)),
                            routing_metadata: HashMap::from([
                                ("channel_id".into(), conversation.channel.clone()),
                                ("user_id".into(), user.clone()),
                            ]),
                            reply_to: advanced::target(&conversation.channel, &thread, &user, team),
                        },
                        action,
                    },
                )))
            }
            _ => Ok(Inbound::Ignore),
        }
    }
}

pub(super) fn ingress_error(error: channelkit::Error) -> ChannelError {
    match error {
        channelkit::Error::Scope | channelkit::Error::Authentication => {
            ChannelError::Unauthorized("Slack authentication or scope failed".into())
        }
        _ => ChannelError::BadRequest("Slack request could not be normalized".into()),
    }
}

/// A scoped identifier is safe to carry through intake; credentials and private
/// file URLs stay in the driver/library. External files are descriptions only.
pub fn provider_attachments(
    context: &SlackEventContext,
    installation: &str,
    team: &str,
    can_download: bool,
) -> Vec<InboundAttachment> {
    context
        .files
        .iter()
        .map(|file| {
            let name = file.name.clone().unwrap_or_else(|| "unnamed file".into());
            if can_download
                && !file.is_external
                && let Some(id) = file.id.as_ref().filter(|id| !id.is_empty())
            {
                InboundAttachment::ProviderFile {
                    reference: serde_json::to_string(&(installation, team, id)).unwrap_or_default(),
                    name: Some(name),
                    media_type: file.mimetype.clone(),
                    size: file.size,
                }
            } else {
                InboundAttachment::FileDescription {
                    name: if file.is_external {
                        file.permalink
                            .as_ref()
                            .map(|url| format!("{name} ({url})"))
                            .unwrap_or(name)
                    } else {
                        name
                    },
                    mime_type: file.mimetype.clone(),
                }
            }
        })
        .collect()
}
