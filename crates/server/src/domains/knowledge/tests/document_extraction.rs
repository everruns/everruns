use super::*;

#[test]
fn metadata_and_binary_are_not_lost() {
    let text = "---\ntype: Custom\ncustom:\n  domain: finance\nverified: []\n---\n# Meaning\n";
    let result = extraction::extract(text.as_bytes(), "note.md", "text/markdown");
    assert_eq!(result.text.as_deref(), Some(text));
    assert_eq!(
        result.metadata["frontmatter"]["custom"]["domain"],
        "finance"
    );
    assert_eq!(
        extraction::extract(b"%PDF\0\x80", "x.pdf", "application/pdf").status,
        "unsupported"
    );
}

#[test]
fn html_uses_markkit_and_retains_extraction_metadata() {
    let extracted=extraction::extract(b"<html><head><title>A page</title></head><body><h1>Guide</h1><p>Useful text</p></body></html>","page.html","text/html");
    assert_eq!(extracted.status, "ready");
    assert!(extracted.text.unwrap().contains("Useful text"));
    assert!(extracted.metadata.get("extraction").is_some());
}

#[test]
fn binary_docx_uses_markkit_for_a_searchable_text_view() {
    let doc = include_bytes!("../../../../testdata/knowledge/minimal.docx");
    let extracted = extraction::extract(
        doc,
        "policy.docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    );
    assert_eq!(extracted.status, "ready");
    assert!(
        extracted
            .text
            .unwrap()
            .contains("Refund policy from a binary Word document")
    );
}

#[test]
fn malformed_ole2_with_impossible_fat_count_is_rejected_without_a_text_view() {
    let mut bytes = vec![0u8; 512];
    bytes[..8].copy_from_slice(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
    bytes[26..28].copy_from_slice(&3u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&0xfffeu16.to_le_bytes());
    bytes[30..32].copy_from_slice(&9u16.to_le_bytes());
    bytes[32..34].copy_from_slice(&6u16.to_le_bytes());
    bytes[44..48].copy_from_slice(&u32::MAX.to_le_bytes());
    let extracted = extraction::extract(&bytes, "malformed.xls", "application/vnd.ms-excel");
    assert_eq!(extracted.status, "failed");
    assert!(extracted.text.is_none());
    assert_eq!(extracted.warnings, vec!["document_extraction_failed"]);
}

#[test]
fn docx_nonfatal_warnings_remain_stable_codes() {
    use std::io::{Cursor, Write};
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    archive.start_file("[Content_Types].xml", options).unwrap();
    archive.write_all(br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#).unwrap();
    archive.start_file("word/document.xml", options).unwrap();
    archive.write_all(br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Visible policy</w:t></w:r><w:r><w:rPr><w:vanish/></w:rPr><w:t>Hidden fixture text</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
    let bytes = archive.finish().unwrap().into_inner();
    let extracted = extraction::extract(
        &bytes,
        "policy.docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    );
    assert_eq!(extracted.status, "ready");
    let text = extracted.text.unwrap();
    assert!(text.contains("Visible policy"));
    assert!(!text.contains("Hidden fixture text"));
    assert_eq!(extracted.warnings, vec!["hidden_content_skipped"]);
}

#[tokio::test]
async fn extractor_upgrade_versions_the_view_and_preserves_original_history() {
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    let bytes = b"<html><body><p>Original policy</p></body></html>";
    let first = write(&s, &c, "/policy.html", bytes).await;
    let first_version = Uuid::parse_str(first["version_id"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE knowledge_versions SET metadata=jsonb_set(metadata,'{extractor}',to_jsonb($2::text)) WHERE id=$1")
        .bind(first_version).bind("markkit-0.1.1/knowledge-v1")
        .execute(s.db.pool()).await.unwrap();
    let next = write(&s, &c, "/policy.html", bytes).await;
    assert_eq!(next["changed"], true);
    assert_ne!(next["version_id"], first["version_id"]);
    let (old, original) = s
        .read_bytes(
            &c,
            "/policy.html",
            &PageQuery {
                version: Some(first_version),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let (current, current_original) = s
        .read_bytes(&c, "/policy.html", &PageQuery::default())
        .await
        .unwrap();
    assert_eq!(original, bytes);
    assert_eq!(current_original, bytes);
    assert_eq!(old.content_hash, current.content_hash);
    assert_eq!(old.metadata["extractor"], "markkit-0.1.1/knowledge-v1");
    assert_eq!(current.metadata["extractor"], extraction::PROFILE);
    let unchanged = write(&s, &c, "/policy.html", bytes).await;
    assert_eq!(unchanged["changed"], false);
    assert_eq!(unchanged["version_id"], next["version_id"]);
}
