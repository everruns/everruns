use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use everruns_contracts::runtime::egress::{
    EgressError, EgressResponse, EgressResult, EgressStreamResponse,
};

use super::*;

/// Stores every file it is given as a text part naming it.
#[derive(Default)]
struct TextStore {
    stored: Mutex<Vec<ReceivedAttachment>>,
}

#[async_trait]
impl AttachmentStore for TextStore {
    async fn store(&self, attachment: ReceivedAttachment) -> Result<ContentPart, ChannelError> {
        let part = ContentPart::text(format!(
            "stored {} ({})",
            attachment.name, attachment.media_type
        ));
        self.stored.lock().unwrap().push(attachment);
        Ok(part)
    }
}

/// A canned response: status, headers and body.
type FakeResponse = (u16, Vec<(String, String)>, Vec<u8>);

/// Answers each URL from a table and records the requests it saw.
#[derive(Default)]
struct FakeEgress {
    responses: BTreeMap<String, FakeResponse>,
    seen: Mutex<Vec<(String, Option<String>)>>,
}

impl FakeEgress {
    fn with(mut self, url: &str, status: u16, headers: &[(&str, &str)], body: &[u8]) -> Self {
        self.responses.insert(
            url.to_string(),
            (
                status,
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body.to_vec(),
            ),
        );
        self
    }
}

#[async_trait]
impl EgressService for FakeEgress {
    async fn send(&self, _: EgressRequest) -> EgressResult<EgressResponse> {
        unreachable!("downloads stream")
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        assert!(request.dns_pinning_required);
        self.seen.lock().unwrap().push((
            request.url.clone(),
            request.headers.get("authorization").cloned(),
        ));
        let Some((status, headers, body)) = self.responses.get(&request.url).cloned() else {
            return Err(EgressError::Transport("unknown host".into()));
        };
        // Two chunks, so the size check runs while reading.
        let half = body.len() / 2;
        let chunks = vec![Ok(body[..half].to_vec()), Ok(body[half..].to_vec())];
        Ok(EgressStreamResponse {
            status,
            headers: headers.into_iter().collect(),
            body: Box::pin(futures::stream::iter(chunks)),
        })
    }
}

fn remote(url: &str, credential: Option<RemoteCredential>) -> InboundAttachment {
    InboundAttachment::Remote {
        url: url.to_string(),
        name: None,
        media_type: Some("application/pdf".into()),
        size: None,
        credential,
    }
}

async fn take(
    egress: Option<Arc<dyn EgressService>>,
    policy: AttachmentPolicy,
    attachments: &[InboundAttachment],
) -> (Vec<String>, Vec<ReceivedAttachment>) {
    let store = TextStore::default();
    let intake = AttachmentIntake {
        store: &store,
        egress,
        policy,
        source: "slack",
    };
    let parts = intake
        .parts(attachments)
        .await
        .into_iter()
        .map(|part| match part {
            ContentPart::Text(text) => text.text,
            other => panic!("unexpected part {other:?}"),
        })
        .collect();
    (parts, store.stored.into_inner().unwrap())
}

#[tokio::test]
async fn remote_file_is_downloaded_with_its_credential_and_stored() {
    let egress = Arc::new(FakeEgress::default().with(
        "https://files.slack.com/F1/report.pdf",
        200,
        &[],
        b"%PDF-1.7 body",
    ));
    let (parts, stored) = take(
        Some(egress.clone()),
        AttachmentPolicy::default(),
        &[remote(
            "https://files.slack.com/F1/report.pdf",
            Some(RemoteCredential::bearer("files.slack.com", "xoxb-1")),
        )],
    )
    .await;

    assert_eq!(parts, ["stored report.pdf (application/pdf)"]);
    assert_eq!(stored[0].data, b"%PDF-1.7 body");
    assert_eq!(
        egress.seen.lock().unwrap()[0].1.as_deref(),
        Some("Bearer xoxb-1")
    );
}

#[tokio::test]
async fn credential_is_not_sent_where_a_redirect_points() {
    let egress = Arc::new(
        FakeEgress::default()
            .with(
                "https://files.slack.com/F1/a.pdf",
                302,
                &[("Location", "https://cdn.example.com/a.pdf")],
                b"",
            )
            .with("https://cdn.example.com/a.pdf", 200, &[], b"%PDF-1.7"),
    );
    let (parts, _) = take(
        Some(egress.clone()),
        AttachmentPolicy::default(),
        &[remote(
            "https://files.slack.com/F1/a.pdf",
            Some(RemoteCredential::bearer("files.slack.com", "xoxb-1")),
        )],
    )
    .await;

    assert_eq!(parts, ["stored a.pdf (application/pdf)"]);
    let seen = egress.seen.lock().unwrap();
    assert_eq!(seen[0].1.as_deref(), Some("Bearer xoxb-1"));
    assert_eq!(seen[1], ("https://cdn.example.com/a.pdf".to_string(), None));
}

#[tokio::test]
async fn files_that_cannot_be_taken_become_notes() {
    let egress: Arc<dyn EgressService> = Arc::new(
        FakeEgress::default()
            .with("https://example.com/big.pdf", 200, &[], &[b'x'; 64])
            .with("https://example.com/gone.pdf", 403, &[], b""),
    );
    let policy = AttachmentPolicy {
        max_files: 4,
        max_bytes: 32,
    };
    let (parts, stored) = take(
        Some(egress),
        policy,
        &[
            remote("https://example.com/big.pdf", None),
            remote("https://example.com/gone.pdf", None),
            remote("http://example.com/plain.pdf", None),
            InboundAttachment::Remote {
                url: "https://example.com/huge.zip".into(),
                name: Some("huge.zip".into()),
                media_type: None,
                size: Some(64),
                credential: None,
            },
            InboundAttachment::Bytes {
                name: Some("fifth.txt".into()),
                media_type: Some("text/plain".into()),
                data: b"hi".to_vec(),
            },
        ],
    )
    .await;

    assert_eq!(
        parts,
        [
            "[Attachment big.pdf not received: 0.0 MB is over the 0.0 MB limit]",
            "[Attachment gone.pdf not received: the download failed (HTTP 403)]",
            "[Attachment plain.pdf not received: its link is not a public https URL]",
            "[Attachment huge.zip not received: 0.0 MB is over the 0.0 MB limit]",
            "[Attachment fifth.txt not received: a message keeps at most 4 files]",
        ]
    );
    assert!(stored.is_empty());
}

#[tokio::test]
async fn bytes_get_their_media_type_from_the_content() {
    let (parts, stored) = take(
        None,
        AttachmentPolicy::default(),
        &[InboundAttachment::Bytes {
            name: Some("photo.png".into()),
            media_type: Some("image/png".into()),
            data: b"%PDF-1.7".to_vec(),
        }],
    )
    .await;

    assert_eq!(parts, ["stored photo.png (application/pdf)"]);
    assert_eq!(stored.len(), 1);
}

#[tokio::test]
async fn a_host_without_egress_notes_remote_files() {
    let (parts, _) = take(
        None,
        AttachmentPolicy::default(),
        &[remote("https://files.slack.com/F1/a.pdf", None)],
    )
    .await;

    assert_eq!(
        parts,
        ["[Attachment a.pdf not received: this host does not download files]"]
    );
}
