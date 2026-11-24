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
        downloads: true,
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

#[tokio::test]
async fn the_anonymous_policy_takes_inline_files_only() {
    let egress: Arc<dyn EgressService> =
        Arc::new(FakeEgress::default().with("https://example.com/a.pdf", 200, &[], b"%PDF-1.7"));
    let inline = |name: &str| InboundAttachment::Bytes {
        name: Some(name.into()),
        media_type: Some("text/plain".into()),
        data: b"hi".to_vec(),
    };
    let (parts, stored) = take(
        Some(egress),
        AttachmentPolicy::anonymous(),
        &[
            remote("https://example.com/a.pdf", None),
            inline("b.txt"),
            inline("c.txt"),
            inline("d.txt"),
        ],
    )
    .await;

    assert_eq!(
        parts,
        [
            "[Attachment a.pdf not received: this channel takes files inline, not as links]",
            "stored b.txt (text/plain)",
            "stored c.txt (text/plain)",
            "[Attachment d.txt not received: a message keeps at most 3 files]",
        ]
    );
    assert_eq!(stored.len(), 2);
}

struct ProviderFiles {
    seen: Mutex<Vec<(String, usize)>>,
    data: Vec<u8>,
}
#[async_trait]
impl super::super::ChannelAttachmentResolver for ProviderFiles {
    async fn download(
        &self,
        reference: &str,
        max_bytes: usize,
        _egress: Arc<dyn EgressService>,
    ) -> Result<Vec<u8>, ChannelError> {
        self.seen
            .lock()
            .unwrap()
            .push((reference.into(), max_bytes));
        Ok(self.data.clone())
    }
}
fn provider_file(reference: &str, size: Option<u64>) -> InboundAttachment {
    InboundAttachment::ProviderFile {
        reference: reference.into(),
        name: Some("report.pdf".into()),
        media_type: Some("image/png".into()),
        size,
    }
}
#[tokio::test]
async fn provider_files_obey_policy_before_credentialed_resolution() {
    let resolver = ProviderFiles {
        seen: Mutex::new(vec![]),
        data: b"%PDF-1.7 body".to_vec(),
    };
    let store = TextStore::default();
    let mut intake = AttachmentIntake {
        store: &store,
        egress: Some(Arc::new(FakeEgress::default())),
        policy: AttachmentPolicy {
            max_files: 1,
            max_bytes: 32,
            downloads: true,
        },
        source: "fixture",
    };
    let parts = intake
        .parts_with_resolver(
            &[
                provider_file("scoped-file", Some(12)),
                provider_file("skipped-file", Some(12)),
            ],
            Some(&resolver),
        )
        .await;
    assert_eq!(parts.len(), 2);
    assert_eq!(
        resolver.seen.lock().unwrap().as_slice(),
        &[("scoped-file".into(), 32)]
    );
    assert_eq!(
        store.stored.lock().unwrap()[0].media_type,
        "application/pdf"
    );
    for file in [
        provider_file("too-large", Some(33)),
        provider_file("https://private.invalid/file", None),
        provider_file("", None),
    ] {
        intake.parts_with_resolver(&[file], Some(&resolver)).await;
    }
    intake.policy.downloads = false;
    intake
        .parts_with_resolver(&[provider_file("disabled", None)], Some(&resolver))
        .await;
    assert_eq!(resolver.seen.lock().unwrap().len(), 1);
    assert_eq!(store.stored.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn provider_bytes_are_checked_and_missing_resolvers_do_not_fallback_to_urls() {
    let resolver = ProviderFiles {
        seen: Mutex::new(vec![]),
        data: vec![0; 33],
    };
    let store = TextStore::default();
    let intake = AttachmentIntake {
        store: &store,
        egress: Some(Arc::new(FakeEgress::default())),
        policy: AttachmentPolicy {
            max_files: 1,
            max_bytes: 32,
            downloads: true,
        },
        source: "fixture",
    };
    let files = [provider_file("scoped-file", None)];
    let oversized = intake.parts_with_resolver(&files, Some(&resolver)).await;
    assert!(matches!(&oversized[0],ContentPart::Text(part) if part.text.contains("not received")));
    let unsupported = intake.parts(&files).await;
    assert!(
        matches!(&unsupported[0],ContentPart::Text(part) if part.text.contains("does not resolve"))
    );
    assert!(store.stored.lock().unwrap().is_empty());
    assert_eq!(resolver.seen.lock().unwrap().len(), 1);
}
struct StalledFiles;
#[async_trait]
impl super::super::ChannelAttachmentResolver for StalledFiles {
    async fn download(
        &self,
        _: &str,
        _: usize,
        _egress: Arc<dyn EgressService>,
    ) -> Result<Vec<u8>, ChannelError> {
        std::future::pending().await
    }
}
#[tokio::test(start_paused = true)]
async fn provider_resolution_obeys_the_intake_deadline() {
    let store = TextStore::default();
    let intake = AttachmentIntake {
        store: &store,
        egress: Some(Arc::new(FakeEgress::default())),
        policy: AttachmentPolicy::default(),
        source: "fixture",
    };
    let parts = intake
        .parts_with_resolver(&[provider_file("scoped-file", None)], Some(&StalledFiles))
        .await;
    assert!(matches!(&parts[0],ContentPart::Text(part) if part.text.contains("timed out")));
    assert!(store.stored.lock().unwrap().is_empty());
}

#[tokio::test]
async fn provider_files_are_not_fetched_without_a_host_transport() {
    let resolver = ProviderFiles {
        seen: Mutex::new(vec![]),
        data: b"%PDF-1.7 body".to_vec(),
    };
    let store = TextStore::default();
    let intake = AttachmentIntake {
        store: &store,
        egress: None,
        policy: AttachmentPolicy::default(),
        source: "fixture",
    };
    let parts = intake
        .parts_with_resolver(&[provider_file("scoped-file", None)], Some(&resolver))
        .await;
    assert!(matches!(&parts[0],ContentPart::Text(part) if part.text.contains("does not download")));
    assert!(resolver.seen.lock().unwrap().is_empty());
    assert!(store.stored.lock().unwrap().is_empty());
}
