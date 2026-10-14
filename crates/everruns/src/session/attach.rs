//! Files kept with a session: written into its files at
//! `/attachments/<id>/<name>`, read-only, and sent to the model by reference,
//! so the agent can open them again after compaction.

use everruns_contracts::runtime::attachments::verified_media_type;
use everruns_core::ContentPart;
use tokio::sync::oneshot;

use super::{Command, RunError, Session, SessionActor};

/// A file for [`Session::attach`].
pub(super) struct AttachRequest {
    name: String,
    media_type: String,
    data: Vec<u8>,
}

/// Why [`Session::attach`] did not keep a file.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum AttachError {
    /// The file itself was refused, such as a zip that cannot be unpacked.
    /// The reason is short and fit to show the model.
    Refused(String),
    /// The session could not take it.
    Run(RunError),
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttachError::Refused(reason) => write!(f, "file not kept: {reason}"),
            AttachError::Run(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for AttachError {}

impl From<RunError> for AttachError {
    fn from(error: RunError) -> Self {
        AttachError::Run(error)
    }
}

impl Session {
    /// Keep a file in this session's files and return the part that sends it
    /// with a message.
    ///
    /// The file is written read-only to `/attachments/<id>/<name>`; the model
    /// gets it with the message (images and PDFs as content, other files by
    /// name and path) and can open it again with `read_file`, including after
    /// compaction drops it from the request. A zip is unpacked into a folder
    /// next to it. `media_type` is what the sender declared; the bytes decide.
    ///
    /// ```no_run
    /// # async fn demo(session: everruns::Session) -> Result<(), Box<dyn std::error::Error>> {
    /// use everruns::{ContentPart, InputMessage};
    ///
    /// let report = session
    ///     .attach("report.pdf", Some("application/pdf"), std::fs::read("report.pdf")?)
    ///     .await?;
    /// let mut message = InputMessage::user("Summarize the attached report.");
    /// message.content.push(report);
    /// session.run(message).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Stability: alpha.
    pub async fn attach(
        &self,
        name: impl Into<String>,
        media_type: Option<&str>,
        data: Vec<u8>,
    ) -> Result<ContentPart, AttachError> {
        let request = AttachRequest {
            name: name.into(),
            media_type: verified_media_type(media_type, &data),
            data,
        };
        let (response, result) = oneshot::channel();
        self.command_sender()
            .await?
            .send(Command::Attach {
                request: Box::new(request),
                response,
            })
            .await
            .map_err(|_| RunError::SessionClosed)?;
        result.await.map_err(|_| RunError::SessionClosed)?
    }
}

impl SessionActor {
    pub(super) async fn attach(
        &mut self,
        request: Box<AttachRequest>,
    ) -> Result<ContentPart, AttachError> {
        self.ensure_runtime().await?;
        self.runtime
            .as_ref()
            .expect("runtime built above")
            .stage_attachment(
                self.session_id,
                &request.name,
                &request.media_type,
                &request.data,
            )
            .await
            .map_err(AttachError::Refused)
    }
}

#[cfg(test)]
#[path = "attach_tests.rs"]
mod tests;
