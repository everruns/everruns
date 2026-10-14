use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;

use super::*;
use crate::host::in_memory::InMemorySessionFileStore;

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[tokio::test]
async fn a_staged_pdf_is_a_read_only_file_the_resolver_reads_back() {
    let store: Arc<dyn SessionFileSystem> = Arc::new(InMemorySessionFileStore::new());
    let session = SessionId::new();

    let part = stage_attachment(
        store.as_ref(),
        session,
        "Q3 report.pdf",
        "application/pdf",
        b"%PDF-1.7 body",
    )
    .await
    .unwrap();

    let ContentPart::File(file) = part else {
        panic!("expected a file part, got {part:?}")
    };
    let path = attachment_path(&file.file_id.to_string(), Some("Q3 report.pdf"), "file");
    let stored = store.read_file(session, &path).await.unwrap().unwrap();
    assert!(stored.is_readonly);

    let resolved = SessionAttachmentFiles::new(store, session)
        .resolve_files(&[file.file_id.uuid()])
        .await
        .unwrap();
    let resolved = &resolved[&file.file_id.uuid()];
    assert_eq!(resolved.media_type, "application/pdf");
    assert_eq!(resolved.filename.as_deref(), Some("Q3_report.pdf"));
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&resolved.base64)
            .unwrap(),
        b"%PDF-1.7 body"
    );
}

#[tokio::test]
async fn a_staged_image_resolves_as_an_image() {
    let store: Arc<dyn SessionFileSystem> = Arc::new(InMemorySessionFileStore::new());
    let session = SessionId::new();
    let png = b"\x89PNG\r\n\x1a\n rest";

    let part = stage_attachment(store.as_ref(), session, "chart.png", "image/png", png)
        .await
        .unwrap();

    let ContentPart::ImageFile(image) = part else {
        panic!("expected an image part, got {part:?}")
    };
    let resolved = SessionAttachmentFiles::new(store, session)
        .resolve_image(image.image_id.uuid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.media_type, "image/png");
}

#[tokio::test]
async fn a_zip_unpacks_next_to_itself_and_a_broken_one_is_refused() {
    let store: Arc<dyn SessionFileSystem> = Arc::new(InMemorySessionFileStore::new());
    let session = SessionId::new();
    let bytes = zip_of(&[("data/report.csv", b"a,b\n1,2\n")]);

    let part = stage_attachment(store.as_ref(), session, "data.zip", ZIP_MEDIA_TYPE, &bytes)
        .await
        .unwrap();

    let ContentPart::File(file) = part else {
        panic!("expected a file part")
    };
    let dir = unpacked_dir(&attachment_path(
        &file.file_id.to_string(),
        Some("data.zip"),
        "file",
    ));
    let unpacked = store
        .read_file(session, &format!("{dir}/data/report.csv"))
        .await
        .unwrap()
        .unwrap();
    assert!(unpacked.is_readonly);

    let error = stage_attachment(
        store.as_ref(),
        session,
        "bad.zip",
        ZIP_MEDIA_TYPE,
        b"PK\x03\x04 not really",
    )
    .await
    .unwrap_err();
    assert!(error.starts_with("the zip cannot be unpacked"), "{error}");
}

#[tokio::test]
async fn an_unknown_id_resolves_to_nothing() {
    let store: Arc<dyn SessionFileSystem> = Arc::new(InMemorySessionFileStore::new());
    let resolver = SessionAttachmentFiles::new(store, SessionId::new());

    assert!(
        resolver
            .resolve_image(ImageId::new().uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        resolver
            .resolve_files(&[FileId::new().uuid()])
            .await
            .unwrap()
            .is_empty()
    );
}
