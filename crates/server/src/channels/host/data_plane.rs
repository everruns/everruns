//! A channel's session on a v2 data plane (data plane v2, step 11.5).
//!
//! The host creates the session as for any org (the control plane creates
//! it on the data plane, `data_plane::sessions`); what changes is where its
//! messages and events live. This module is the port's path for those.
//!
//! Decisions:
//! - A message is admitted on the data plane with its metadata (the
//!   platform's thread and channel ids) and its sender, under a fresh id.
//!   The cell answers whether it started a turn or steered the running one,
//!   which is what the host needs to know whether this message owes a reply.
//! - Text only, as the v2 chat is: a picture or file a channel received is
//!   named in a note instead (attachments on v2 come with the 11.6 sweep).
//! - Events are the session's stream on the data plane, followed from the
//!   last sequence seen when the data plane cycles the connection, so the
//!   host sees one stream as it does on v1.
//! - History seeded into a new session goes through the data plane's own
//!   seed, which records it only while the session has no conversation yet.
//! - Reactions are not recorded on a v2 session yet (11.6); a reaction is
//!   logged and dropped rather than refused, so the channel keeps working.

use std::time::Duration;

use everruns_contracts::runtime::channel::ChannelError;
use everruns_contracts::runtime::message::ContentPart;
use everruns_contracts::runtime::message_retriever::InputMessage;
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::RuntimeMessage;
use everruns_core::channel_runtime::{ChannelEventStream, DeliveryEvent, SendOutcome};
use futures::StreamExt;
use serde_json::{Value, json};

use crate::domains::data_plane::{DataPlane, for_org, org_public, sessions};
use crate::domains::messages::MessageService;

/// How long a dropped stream waits before following the session again.
const RECONNECT_AFTER: Duration = Duration::from_millis(200);

/// One of the org's sessions that its data plane runs.
pub(super) struct DpSession {
    plane: std::sync::Arc<dyn DataPlane>,
    org: String,
    session: SessionId,
}

fn other(error: anyhow::Error) -> ChannelError {
    ChannelError::Other(error)
}

/// `session` when the org's data plane runs it; `None` for a session the
/// platform's workers run.
pub(super) async fn find(
    messages: &MessageService,
    org_id: i64,
    session: SessionId,
) -> Result<Option<DpSession>, ChannelError> {
    let cp = messages.data_plane();
    if !sessions::runs_on_data_plane(&cp, org_id, session)
        .await
        .map_err(other)?
    {
        return Ok(None);
    }
    let plane = for_org(&cp, org_id).await.map_err(other)?;
    Ok(Some(DpSession {
        plane: std::sync::Arc::from(plane),
        org: org_public(org_id),
        session,
    }))
}

/// The message's text, with each part a v2 session cannot take yet named.
fn text_of(content: &[ContentPart]) -> String {
    content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(text) => Some(text.text.clone()),
            ContentPart::Image(_) | ContentPart::ImageFile(_) => {
                Some("[An image was attached; this session cannot open it yet.]".into())
            }
            ContentPart::File(_) => {
                Some("[A file was attached; this session cannot open it yet.]".into())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The body of the data plane's `messages` command for `message`.
pub(super) fn message_body(id: MessageId, message: &InputMessage) -> Value {
    json!({
        "id": id,
        "text": text_of(&message.content),
        "metadata": message.metadata,
        "external_actor": message.external_actor,
    })
}

/// What the host makes of the data plane's answer to an admission.
pub(super) fn outcome(id: MessageId, reply: &Value) -> SendOutcome {
    let input_message_id = id.to_string();
    match reply["disposition"].as_str() {
        Some("steered") => SendOutcome::Steered,
        Some("pending") => SendOutcome::Pending { input_message_id },
        _ => SendOutcome::Started { input_message_id },
    }
}

impl DpSession {
    pub(super) async fn send(&self, message: &InputMessage) -> Result<SendOutcome, ChannelError> {
        let id = MessageId::new();
        let reply = self
            .plane
            .session_command(
                &self.org,
                self.session,
                "messages",
                &message_body(id, message),
            )
            .await
            .map_err(|error| other(error.context("the data plane did not take the message")))?;
        Ok(outcome(id, &reply))
    }

    /// Cancel whichever turn runs now; a no-op when none does.
    pub(super) async fn cancel(&self) -> Result<(), ChannelError> {
        self.plane
            .session_command(
                &self.org,
                self.session,
                "cancel-turn",
                &json!({ "command": uuid::Uuid::new_v4() }),
            )
            .await
            .map_err(other)?;
        Ok(())
    }

    pub(super) async fn seed(&self, history: Vec<RuntimeMessage>) -> Result<(), ChannelError> {
        if history.is_empty() {
            return Ok(());
        }
        self.plane
            .session_command(
                &self.org,
                self.session,
                "seed",
                &json!({ "command": uuid::Uuid::new_v4(), "messages": history }),
            )
            .await
            .map_err(other)?;
        Ok(())
    }

    /// The session's events after `after`, then live, followed across the
    /// data plane cycling the connection.
    pub(super) async fn events(
        self,
        after: Option<i64>,
    ) -> Result<ChannelEventStream, ChannelError> {
        // The first connection's failure is the caller's; later ones retry.
        let first = self
            .plane
            .events(&self.org, self.session, after)
            .await
            .map_err(other)?;
        let state = (Some(first), self, after);
        let envelopes = futures::stream::unfold(state, |(mut current, me, mut last)| async move {
            loop {
                let stream = match current.as_mut() {
                    Some(stream) => stream,
                    None => {
                        tokio::time::sleep(RECONNECT_AFTER).await;
                        match me.plane.events(&me.org, me.session, last).await {
                            Ok(stream) => current.insert(stream),
                            Err(error) => {
                                tracing::debug!(session = %me.session, %error, "channel: data plane stream not reached, retrying");
                                continue;
                            }
                        }
                    }
                };
                match stream.next().await {
                    Some(envelope) => {
                        if let Some(sequence) = envelope["sequence"].as_i64() {
                            last = Some(last.map_or(sequence, |l| l.max(sequence)));
                        }
                        return Some((envelope, (current, me, last)));
                    }
                    None => current = None,
                }
            }
        });
        Ok(envelopes
            .filter_map(|envelope| async move { DeliveryEvent::from_envelope(&envelope) })
            .boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_goes_with_its_thread_and_sender_and_names_what_it_cannot_carry() {
        let mut message =
            InputMessage::user("look at this").with_metadata("thread_ts", json!("1700000000.0001"));
        message.content.push(ContentPart::file(
            everruns_contracts::typed_id::FileId::new(),
        ));
        let body = message_body(MessageId::new(), &message);
        assert_eq!(body["metadata"]["thread_ts"], "1700000000.0001");
        let text = body["text"].as_str().unwrap_or_default();
        assert!(text.starts_with("look at this\n["), "{text}");
    }

    #[test]
    fn the_answer_says_whether_a_reply_is_owed() {
        let id = MessageId::new();
        assert!(matches!(
            outcome(id, &json!({ "disposition": "steered" })),
            SendOutcome::Steered
        ));
        assert!(matches!(
            outcome(id, &json!({ "disposition": "started" })),
            SendOutcome::Started { .. }
        ));
        assert!(matches!(
            outcome(id, &json!({ "disposition": "pending" })),
            SendOutcome::Pending { .. }
        ));
    }
}
