//! Attachment intake: files a platform sent with a message become stored
//! attachments, or a note saying why they did not.
//!
//! Decision: every channel hands files to the host the same way, as bytes or
//! a URL to download ([`InboundAttachment::Bytes`], [`InboundAttachment::Remote`]).
//! The host downloads through its egress service (https only, DNS-pinned, a
//! credential only to its own host, at most [`MAX_DOWNLOAD_REDIRECTS`]
//! redirects), checks the media type against the bytes, applies the
//! [`AttachmentPolicy`], and asks an [`AttachmentStore`] to store each file. A
//! file that cannot be taken becomes a text note in the message, so the agent
//! knows it was sent; the reason the platform or storage gave goes to the log.
//!
//! [`AttachmentIntake`] is public so surfaces that are not channels (the agent
//! execution API, MCP tools) take files the same way, with their own store.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime::attachments::verified_media_type;
use everruns_contracts::url_validation::validate_safe_url;
use futures::StreamExt;
use tracing::warn;

use everruns_contracts::runtime::egress::{EgressRequest, EgressRequestKind, EgressService};
use everruns_contracts::runtime::message::ContentPart;

use super::{ChannelError, InboundAttachment, RemoteCredential};

/// Most redirects one download follows.
pub const MAX_DOWNLOAD_REDIRECTS: usize = 3;

/// How long one download may take, redirects included.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// What a host takes from one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentPolicy {
    /// Most files kept from one message; the rest become notes.
    pub max_files: usize,
    /// Largest file kept, in bytes.
    pub max_bytes: u64,
}

impl Default for AttachmentPolicy {
    fn default() -> Self {
        Self {
            max_files: 10,
            max_bytes: 25 * 1024 * 1024,
        }
    }
}

/// A file the host received, with its media type checked against the bytes,
/// for an [`AttachmentStore`] to store.
#[derive(Debug, Clone)]
pub struct ReceivedAttachment {
    pub name: String,
    pub media_type: String,
    pub data: Vec<u8>,
}

/// Why a file was not taken: shown to the model in the note, so it is short
/// and holds nothing secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NotReceived(pub String);

/// The note that stands in for a file the host could not take.
pub fn not_received_note(name: &str, reason: &str) -> String {
    format!("[Attachment {name} not received: {reason}]")
}

/// Keeps a received file and returns the part that refers to it.
#[async_trait]
pub trait AttachmentStore: Send + Sync {
    /// Store `attachment`. A `BadRequest` reason reaches the agent in the
    /// note; any other error is logged and the note says it was not stored.
    async fn store(&self, attachment: ReceivedAttachment) -> Result<ContentPart, ChannelError>;
}

/// Takes the files of one message: downloads, checks, stores.
pub struct AttachmentIntake<'a> {
    /// Where each file is kept.
    pub store: &'a dyn AttachmentStore,
    /// Downloads `Remote` files; without it they become notes.
    pub egress: Option<Arc<dyn EgressService>>,
    /// Limits per message.
    pub policy: AttachmentPolicy,
    /// Names the surface in logs (a channel name, `api`, `mcp`).
    pub source: &'a str,
}

impl AttachmentIntake<'_> {
    /// Content parts for a message's files, in order: each a stored
    /// attachment or a note.
    pub async fn parts(&self, attachments: &[InboundAttachment]) -> Vec<ContentPart> {
        let mut parts = Vec::new();
        let mut files = 0;
        for attachment in attachments {
            match attachment {
                InboundAttachment::Image { url, .. } => parts.push(ContentPart::image_url(url)),
                InboundAttachment::FileDescription { name, mime_type } => {
                    parts.push(ContentPart::text(match mime_type {
                        Some(mime) => format!("[Attached file: {name} ({mime})]"),
                        None => format!("[Attached file: {name}]"),
                    }))
                }
                InboundAttachment::Bytes { .. } | InboundAttachment::Remote { .. } => {
                    let name = display_name(attachment);
                    files += 1;
                    let part = if files > self.policy.max_files {
                        Err(NotReceived(format!(
                            "a message keeps at most {} files",
                            self.policy.max_files
                        )))
                    } else {
                        self.take(attachment, &name).await
                    };
                    parts.push(part.unwrap_or_else(|NotReceived(reason)| {
                        ContentPart::text(not_received_note(&name, &reason))
                    }));
                }
            }
        }
        parts
    }

    async fn take(
        &self,
        attachment: &InboundAttachment,
        name: &str,
    ) -> Result<ContentPart, NotReceived> {
        let (declared, data) = match attachment {
            InboundAttachment::Bytes {
                media_type, data, ..
            } => {
                self.check_size(data.len() as u64)?;
                (media_type.as_deref(), data.clone())
            }
            InboundAttachment::Remote {
                url,
                media_type,
                size,
                credential,
                ..
            } => {
                if let Some(size) = size {
                    self.check_size(*size)?;
                }
                let data = self.download(url, credential.as_ref()).await?;
                (media_type.as_deref(), data)
            }
            _ => unreachable!("only files are taken"),
        };
        if data.is_empty() {
            return Err(NotReceived("the file is empty".into()));
        }
        let received = ReceivedAttachment {
            name: name.to_string(),
            media_type: verified_media_type(declared, &data),
            data,
        };
        self.store.store(received).await.map_err(|error| {
            warn!(source = %self.source, file = %name, %error, "attachment not stored");
            NotReceived(match error {
                ChannelError::BadRequest(reason) => reason,
                _ => "it could not be stored".into(),
            })
        })
    }

    fn check_size(&self, size: u64) -> Result<(), NotReceived> {
        if size > self.policy.max_bytes {
            return Err(NotReceived(format!(
                "{} is over the {} limit",
                megabytes(size),
                megabytes(self.policy.max_bytes)
            )));
        }
        Ok(())
    }

    async fn download(
        &self,
        url: &str,
        credential: Option<&RemoteCredential>,
    ) -> Result<Vec<u8>, NotReceived> {
        let Some(egress) = &self.egress else {
            return Err(NotReceived("this host does not download files".into()));
        };
        match tokio::time::timeout(DOWNLOAD_TIMEOUT, self.fetch(egress, url, credential)).await {
            Ok(result) => result,
            Err(_) => Err(NotReceived("the download timed out".into())),
        }
    }

    async fn fetch(
        &self,
        egress: &Arc<dyn EgressService>,
        url: &str,
        credential: Option<&RemoteCredential>,
    ) -> Result<Vec<u8>, NotReceived> {
        let mut url = url.to_string();
        for _ in 0..=MAX_DOWNLOAD_REDIRECTS {
            let parsed = validate_safe_url(&url)
                .ok()
                .filter(|parsed| parsed.scheme() == "https")
                .ok_or_else(|| NotReceived("its link is not a public https URL".into()))?;
            let mut request =
                EgressRequest::new("GET", url.clone(), EgressRequestKind::Integration)
                    .timeout_ms(DOWNLOAD_TIMEOUT.as_millis() as u64)
                    .require_dns_pinning();
            // A credential goes only to the host it belongs to, never to
            // wherever a link or a redirect points.
            if let Some(credential) = credential
                && parsed
                    .host_str()
                    .is_some_and(|host| host.eq_ignore_ascii_case(&credential.host))
            {
                request = request.header("authorization", credential.authorization.clone());
            }
            let response = egress.send_stream(request).await.map_err(|error| {
                warn!(host = ?parsed.host_str(), %error, "attachment download failed");
                NotReceived("the download failed".into())
            })?;
            if (300..400).contains(&response.status) {
                let location = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value.clone())
                    .ok_or_else(|| NotReceived("the download failed".into()))?;
                url = parsed
                    .join(&location)
                    .map_err(|_| NotReceived("the download failed".into()))?
                    .to_string();
                continue;
            }
            if !(200..300).contains(&response.status) {
                warn!(host = ?parsed.host_str(), status = response.status, "attachment download refused");
                return Err(NotReceived(format!(
                    "the download failed (HTTP {})",
                    response.status
                )));
            }
            let mut body = response.body;
            let mut data = Vec::new();
            while let Some(chunk) = body.next().await {
                let chunk = chunk.map_err(|_| NotReceived("the download failed".into()))?;
                data.extend_from_slice(&chunk);
                self.check_size(data.len() as u64)?;
            }
            return Ok(data);
        }
        Err(NotReceived("the download redirected too often".into()))
    }
}

fn display_name(attachment: &InboundAttachment) -> String {
    let (name, url) = match attachment {
        InboundAttachment::Bytes { name, .. } => (name.as_deref(), None),
        InboundAttachment::Remote { name, url, .. } => (name.as_deref(), Some(url.as_str())),
        _ => (None, None),
    };
    name.map(str::to_string)
        .or_else(|| {
            let url = url::Url::parse(url?).ok()?;
            let last = url.path_segments()?.next_back()?.to_string();
            (!last.is_empty()).then_some(last)
        })
        .unwrap_or_else(|| "file".to_string())
}

fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 10.0 {
        format!("{mb:.0} MB")
    } else {
        format!("{mb:.1} MB")
    }
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
