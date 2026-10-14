//! Files sent with a message to `POST /v1/sessions/{id}/messages`.
//!
//! Decision: serve takes the same `files` list as the server's Agent API
//! (inline base64 `data` or a public `url`) through the shared intake, and
//! keeps each file in the session's own files at `/attachments/<id>/<name>`
//! with `Session::attach`, exactly as a channel's files are kept. A file that
//! cannot be taken becomes a note the agent reads. Downloads go through the
//! runtime egress, the one agents' tools use.

use async_trait::async_trait;
use everruns::channels::{
    AttachmentIntake, AttachmentPolicy, AttachmentStore, ChannelError, InboundAttachment,
    ReceivedAttachment,
};
use everruns::execution_api::MessageFile;
use everruns::{AttachError, ContentPart, Session};

/// Keeps files in one session.
struct SessionStore(Session);

#[async_trait]
impl AttachmentStore for SessionStore {
    async fn store(&self, attachment: ReceivedAttachment) -> Result<ContentPart, ChannelError> {
        self.0
            .attach(
                attachment.name,
                Some(&attachment.media_type),
                attachment.data,
            )
            .await
            .map_err(|error| match error {
                AttachError::Refused(reason) => ChannelError::BadRequest(reason),
                other => ChannelError::Session(other.to_string()),
            })
    }
}

/// Content parts for `files`, in order: each kept in `session`'s files, or a
/// note saying why not.
pub(crate) async fn receive(session: Session, files: Vec<MessageFile>) -> Vec<ContentPart> {
    if files.is_empty() {
        return Vec::new();
    }
    let attachments: Vec<_> = files.into_iter().map(InboundAttachment::from).collect();
    AttachmentIntake {
        store: &SessionStore(session),
        egress: Some(everruns::batteries::runtime_egress_service()),
        policy: AttachmentPolicy::default(),
        source: "api",
    }
    .parts(&attachments)
    .await
}
