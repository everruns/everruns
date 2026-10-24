use super::*;
use everruns_contracts::knowledge::{IndexMode, KnowledgeQuery};
use serde_json::json;
use types::*;

fn fixture() -> KnowledgeService {
    KnowledgeService::files(Arc::new(crate::storage::StorageBackend::test_database()))
}
async fn create(service: &KnowledgeService, indexing: IndexMode) -> Collection {
    let result = service
        .create(
            1,
            CollectionInput {
                name: format!("test-{}", Uuid::new_v4()),
                indexing,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    service
        .collection(1, result["id"].as_str().unwrap())
        .await
        .unwrap()
}
async fn write(
    service: &KnowledgeService,
    c: &Collection,
    p: &str,
    bytes: &[u8],
) -> serde_json::Value {
    let source: Source =
        sqlx::query_as("SELECT * FROM knowledge_sources WHERE collection_id=$1 AND kind='managed'")
            .bind(c.id)
            .fetch_one(service.db.pool())
            .await
            .unwrap();
    let bytes = bytes.to_vec();
    service
        .ingest(
            c,
            &source,
            FileInput {
                path: p.into(),
                metadata: json!({}),
                ..Default::default()
            },
            futures::stream::once(async {
                Ok::<_, std::io::Error>(axum::body::Bytes::from(bytes))
            }),
            format!("knowledge://{}{p}", c.public_id()),
            None,
            None,
            None,
        )
        .await
        .unwrap()
}
async fn drain(service: &KnowledgeService) {
    for _ in 0..50 {
        if !service.run_once().await.unwrap() {
            break;
        }
    }
}
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
fn fragment_ranges_are_utf8_and_complete() {
    let text = "日本語🙂 paragraph\n\n".repeat(1200);
    let ranges = extraction::fragments(&text);
    assert!(ranges.len() > 1);
    let restored = ranges
        .into_iter()
        .map(|(s, n)| &text[s..s + n])
        .collect::<String>();
    assert_eq!(restored, text);
}
#[tokio::test]
async fn unindexed_binary_and_structure_round_trip() {
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    let raw = b"\0\x80binary bytes";
    write(&s, &c, "/assets/diagram.bin", raw).await;
    let (_, bytes) = s
        .read_bytes(&c, "/assets/diagram.bin", &PageQuery::default())
        .await
        .unwrap();
    assert_eq!(bytes, raw);
    let result = s
        .search(
            1,
            KnowledgeQuery {
                query: "binary".into(),
                collections: vec![c.public_id()],
                limit: 5,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(result.results.is_empty());
    assert_eq!(result.warnings.len(), 1);
    let page = s.files_page(&c, &PageQuery::default()).await.unwrap();
    assert_eq!(page["data"][0]["path"], "/assets/diagram.bin");
}
#[tokio::test]
async fn incremental_index_preserves_noop_identity_and_old_citations() {
    let s = fixture();
    let c = create(&s, IndexMode::Lexical).await;
    write(
        &s,
        &c,
        "/guide.md",
        b"# Refunds\n\nRefund period is thirty days.",
    )
    .await;
    drain(&s).await;
    let q = || KnowledgeQuery {
        query: "Refund".into(),
        collections: vec![c.public_id()],
        limit: 5,
        ..Default::default()
    };
    let old = s.search(1, q()).await.unwrap();
    assert_eq!(old.results.len(), 1);
    let unchanged = write(
        &s,
        &c,
        "/guide.md",
        b"# Refunds\n\nRefund period is thirty days.",
    )
    .await;
    assert_eq!(unchanged["changed"], false);
    drain(&s).await;
    assert_eq!(
        s.search(1, q()).await.unwrap().results[0].id,
        old.results[0].id
    );
    write(
        &s,
        &c,
        "/guide.md",
        b"# Refunds\n\nRefund period is sixty days.",
    )
    .await;
    assert!(
        s.search(1, q()).await.unwrap().results[0]
            .snippet
            .contains("thirty")
    );
    drain(&s).await;
    assert!(
        s.search(1, q()).await.unwrap().results[0]
            .snippet
            .contains("sixty")
    );
    let file = Uuid::parse_str(&old.results[0].file_id).unwrap();
    let version = Uuid::parse_str(&old.results[0].version_id).unwrap();
    assert!(s.version(1, file, version).await.is_ok());
}
#[tokio::test]
async fn turning_index_off_preserves_files_and_prevents_retrieval() {
    let s = fixture();
    let c = create(&s, IndexMode::Lexical).await;
    write(&s, &c, "/note.md", b"secret recipe").await;
    drain(&s).await;
    s.update(
        1,
        &c.public_id(),
        CollectionInput {
            name: c.name.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let result = s
        .search(
            1,
            KnowledgeQuery {
                query: "recipe".into(),
                collections: vec![c.public_id()],
                limit: 5,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(result.results.is_empty());
    assert!(
        s.read_bytes(&c, "/note.md", &PageQuery::default())
            .await
            .is_ok()
    );
    assert!(s.collection(999, &c.public_id()).await.is_err());
}
#[tokio::test]
async fn content_ranges_cross_segments_without_loading_whole_file() {
    let s = fixture();
    let store = content::Content::new(s.db.clone());
    let bytes = vec![42u8; content::SEGMENT * 3 + 17];
    let stored = store
        .store(
            1,
            Uuid::now_v7(),
            futures::stream::once(async {
                Ok::<_, std::io::Error>(axum::body::Bytes::from(bytes))
            }),
        )
        .await
        .unwrap();
    let part = store
        .range(
            &stored.key,
            stored.bytes as u64,
            (content::SEGMENT - 10) as u64,
            40,
        )
        .await
        .unwrap();
    assert_eq!(part, vec![42u8; 40]);
}

#[tokio::test]
async fn source_jobs_can_be_queued_again_after_completion() {
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    s.enqueue(&c, "reindex", None, None, None).await.unwrap();
    s.enqueue(&c, "reindex", None, None, None).await.unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM knowledge_jobs WHERE collection_id=$1")
            .bind(c.id)
            .fetch_one(s.db.pool())
            .await
            .unwrap();
    assert_eq!(count, 1);
    drain(&s).await;
    s.enqueue(&c, "reindex", None, None, None).await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM knowledge_jobs WHERE collection_id=$1 AND status='pending'",
    )
    .bind(c.id)
    .fetch_one(s.db.pool())
    .await
    .unwrap();
    assert_eq!(count, 1);
}
#[tokio::test]
async fn mounts_enforce_bindings_exclusions_and_contextual_grep() {
    use crate::storage::CreateSessionRow;
    use everruns_contracts::typed_id::PrincipalId;
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    write(&s, &c, "/guide.md", b"before\nneedle exact\nafter").await;
    write(&s, &c, "/private.md", b"needle secret").await;
    let mut c = c;
    c.rules = json!({"read_exclude":["private.md"]});
    sqlx::query("UPDATE knowledge_collections SET rules=$2 WHERE id=$1")
        .bind(c.id)
        .bind(&c.rules)
        .execute(s.db.pool())
        .await
        .unwrap();
    let session =
        s.db.create_session(CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            capabilities: json!([{"ref":"knowledge","config":{"collections":[c.public_id()]}}]),
            ..Default::default()
        })
        .await
        .unwrap();
    let base = format!("/knowledge/{}", c.public_id());
    let catalog = s
        .mount_read(session.id.uuid(), &format!("{base}/index.md"))
        .await
        .unwrap()
        .unwrap();
    assert!(!catalog.content.unwrap().contains("private.md"));
    assert!(
        s.mount_read(session.id.uuid(), &format!("{base}/private.md"))
            .await
            .is_err()
    );
    let result = s
        .mount_grep_with_options(
            session.id.uuid(),
            "needle",
            &everruns_core::GrepOptions {
                path_pattern: Some(base.clone()),
                before_context: 1,
                after_context: 1,
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.total_matches, 1);
    assert_eq!(result.blocks[0].lines.len(), 3);
    assert!(result.blocks[0].lines[1].is_match);
    let other =
        s.db.create_session(CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        s.mount_read(other.id.uuid(), &format!("{base}/guide.md"))
            .await
            .unwrap()
            .is_none()
    );
    let fs = crate::domains::session_files::service::WorkspaceFileService::new(s.db.clone());
    assert!(
        fs.list_directory(session.id.uuid(), "/")
            .await
            .unwrap()
            .iter()
            .any(|f| f.path == "/knowledge")
    );
}
#[test]
fn html_uses_markkit_and_retains_extraction_metadata() {
    let extracted=extraction::extract(b"<html><head><title>A page</title></head><body><h1>Guide</h1><p>Useful text</p></body></html>","page.html","text/html");
    assert_eq!(extracted.status, "ready");
    assert!(extracted.text.unwrap().contains("Useful text"));
    assert!(extracted.metadata.get("extraction").is_some());
}

#[tokio::test]
async fn large_file_catalog_uses_bounded_stable_cursor_pages() {
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    let source: Uuid = sqlx::query_scalar(
        "SELECT id FROM knowledge_sources WHERE collection_id=$1 AND kind='managed'",
    )
    .bind(c.id)
    .fetch_one(s.db.pool())
    .await
    .unwrap();
    sqlx::query("INSERT INTO knowledge_files(id,org_id,collection_id,source_id,path) SELECT gen_random_uuid(),$1,$2,$3,'/docs/'||lpad(n::text,5,'0')||'.md' FROM generate_series(1,2000) n").bind(c.org_id).bind(c.id).bind(source).execute(s.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO knowledge_versions(id,org_id,file_id,content_hash,size_bytes,mime_type,raw_key,raw_parts,metadata,extraction_status,source_uri) SELECT gen_random_uuid(),org_id,id,'hash',0,'text/markdown','knowledge/test/empty',1,'{}','ready','test://' FROM knowledge_files WHERE collection_id=$1").bind(c.id).execute(s.db.pool()).await.unwrap();
    sqlx::query("UPDATE knowledge_files f SET current_version=v.id FROM knowledge_versions v WHERE v.file_id=f.id AND f.collection_id=$1").bind(c.id).execute(s.db.pool()).await.unwrap();
    let mut after = None;
    let mut paths = std::collections::BTreeSet::new();
    let mut pages = 0;
    loop {
        let result = s
            .files_page(
                &c,
                &PageQuery {
                    limit: Some(137),
                    after,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let data = result["data"].as_array().unwrap();
        assert!(data.len() <= 137);
        for f in data {
            assert!(paths.insert(f["path"].as_str().unwrap().to_owned()));
        }
        pages += 1;
        after = result["next_cursor"].as_str().map(str::to_owned);
        if after.is_none() {
            break;
        }
    }
    assert_eq!(paths.len(), 2000);
    assert_eq!(pages, 15);
}

#[tokio::test]
async fn a_superseded_worker_cannot_publish_a_projection() {
    let s = fixture();
    let c = create(&s, IndexMode::Lexical).await;
    write(&s, &c, "/guide.md", b"publication fence").await;
    let job:Job=sqlx::query_as("UPDATE knowledge_jobs SET status='running',lease_token=$2,lease_until=now()+interval '5 minutes' WHERE collection_id=$1 RETURNING *").bind(c.id).bind(Uuid::now_v7()).fetch_one(s.db.pool()).await.unwrap();
    s.update(
        1,
        &c.public_id(),
        CollectionInput {
            name: c.name.clone(),
            indexing: IndexMode::Lexical,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    s.index_file(&c, &job).await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM knowledge_file_projections WHERE collection_id=$1",
    )
    .bind(c.id)
    .fetch_one(s.db.pool())
    .await
    .unwrap();
    assert_eq!(count, 0);
    drain(&s).await;
    let current = s.collection(1, &c.public_id()).await.unwrap();
    assert_eq!(current.published_revision, current.revision);
}
#[tokio::test]
async fn explicit_unavailable_indexer_fails_closed() {
    let s = fixture();
    let mut c = create(&s, IndexMode::Lexical).await;
    c.indexer_id = Some(Uuid::now_v7());
    assert!(s.backend(&c).await.is_err());
}

#[test]
fn binary_docx_uses_markkit_for_a_searchable_text_view() {
    let doc = include_bytes!("../../../testdata/knowledge/minimal.docx");
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

#[tokio::test]
async fn automatic_disclosure_reuses_evidence_and_observes_read_revocation() {
    use crate::storage::CreateSessionRow;
    use everruns_contracts::typed_id::{MessageId, PrincipalId};
    let s = fixture();
    let c = create(&s, IndexMode::Lexical).await;
    write(&s, &c, "/guide.md", b"Refund period is thirty days.").await;
    drain(&s).await;
    let session=s.db.create_session(CreateSessionRow{org_id:1,owner_principal_id:PrincipalId::from_seed(1),capabilities:json!([{"ref":"knowledge","config":{"collections":[c.public_id()],"automatic":true}}]),..Default::default()}).await.unwrap();
    let input = MessageId::from_uuid(Uuid::now_v7());
    s.db.record_runtime_invocation(1, session.id, input.uuid(), None, None, None)
        .await
        .unwrap();
    let original = s
        .automatic(1, session.id, input, "Refund".into())
        .await
        .unwrap()
        .unwrap();
    assert!(original.contains("thirty"));
    write(&s, &c, "/guide.md", b"Refund period is sixty days.").await;
    drain(&s).await;
    assert_eq!(
        s.automatic(1, session.id, input, "Refund".into())
            .await
            .unwrap()
            .unwrap(),
        original
    );
    sqlx::query("UPDATE knowledge_files SET readable=false WHERE collection_id=$1")
        .bind(c.id)
        .execute(s.db.pool())
        .await
        .unwrap();
    assert!(
        s.automatic(1, session.id, input, "Refund".into())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn indexer_binding_does_not_prevent_credential_revocation() {
    let s = fixture();
    let mut c = create(&s, IndexMode::Lexical).await;
    let credential = Uuid::now_v7();
    let account = Uuid::now_v7();
    sqlx::query("INSERT INTO resource_access(id,org_id,kind,scope,usage) VALUES($1,1,'credential_binding','organization','service')").bind(credential).execute(s.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO credential_bindings(id,org_id,kind,env_name,binding,label) VALUES($1,1,'http','KNOWLEDGE_REVOKE_TEST',$2,'Test')").bind(credential).bind(json!({"url":"https://gcp-us-central1.turbopuffer.com","auth":{"type":"bearer"}})).execute(s.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO knowledge_indexers(id,org_id,name,endpoint,credential_binding_id) VALUES($1,1,'test-revocation','https://gcp-us-central1.turbopuffer.com',$2)").bind(account).bind(credential).execute(s.db.pool()).await.unwrap();
    sqlx::query("UPDATE resource_access SET revoked_at=now(),revision=revision+1 WHERE id=$1")
        .bind(credential)
        .execute(s.db.pool())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query("DELETE FROM credential_bindings WHERE id=$1")
            .bind(credential)
            .execute(s.db.pool())
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    c.indexer_id = Some(account);
    assert!(s.backend(&c).await.is_err());
}

#[tokio::test]
async fn content_updates_preserve_omitted_annotations_and_read_index_policy() {
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    let source: Source = sqlx::query_as("SELECT * FROM knowledge_sources WHERE collection_id=$1")
        .bind(c.id)
        .fetch_one(s.db.pool())
        .await
        .unwrap();
    for (bytes, metadata, indexable, readable) in [
        (
            "original",
            json!({"custom":{"owner":"support"}}),
            Some(false),
            Some(false),
        ),
        ("updated", serde_json::Value::Null, None, None),
    ] {
        let data = bytes.as_bytes().to_vec();
        s.ingest(
            &c,
            &source,
            FileInput {
                path: "/note.md".into(),
                metadata,
                indexable,
                readable,
                ..Default::default()
            },
            futures::stream::once(async { Ok::<_, std::io::Error>(axum::body::Bytes::from(data)) }),
            "test://note".into(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    }
    let f = s.file(&c, "/note.md").await.unwrap();
    assert!(!f.readable);
    assert!(!f.indexable);
    let v = s
        .version(c.org_id, f.id, f.current_version.unwrap())
        .await
        .unwrap();
    assert_eq!(v.metadata["user"]["custom"]["owner"], "support");
}

#[test]
fn okf_frontmatter_handles_bom_crlf_and_exact_fences() {
    let text =
        "\u{feff}---\r\ntype: Custom\r\nextra:\r\n  status: verified\r\n---\r\n# Heading\r\n";
    let extracted = extraction::extract(text.as_bytes(), "guide.md", "text/markdown");
    assert_eq!(extracted.text.as_deref(), Some(text));
    assert_eq!(
        extracted.metadata["frontmatter"]["extra"]["status"],
        "verified"
    );
    assert!(extraction::frontmatter("---\ntype: Test\n---invalid\nbody").is_err());
}

#[tokio::test]
async fn okf_keeps_navigation_unknown_metadata_and_binary_assets() {
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    let files: &[(&str, &[u8])] = &[
        (
            "/index.md",
            b"---\nokf_version: \"0.2\"\n---\n* [Guide](guide.md)\n* [Docs](docs/)\n",
        ),
        ("/log.md", b"# Updates\n\n## 2026-10-23\n\nAdded guide.\n"),
        ("/docs/index.md", b"* [Diagram](diagram.bin)\n"),
        (
            "/guide.md",
            b"---\ntype: Playbook\ncustom:\n  reviewers: [alice, bob]\n---\n# Guide\n",
        ),
        ("/docs/diagram.bin", b"\0\xff\x80original"),
    ];
    for (path, bytes) in files {
        write(&s, &c, path, bytes).await;
    }
    let result = s.validate_okf(&c, &PageQuery::default()).await.unwrap();
    assert_eq!(result["checked"], 4);
    assert_eq!(result["issues"], json!([]));
    for (path, bytes) in files {
        assert_eq!(
            &s.read_bytes(&c, path, &PageQuery::default())
                .await
                .unwrap()
                .1,
            bytes
        );
    }
    write(&s, &c, "/invalid.md", b"# No type\n").await;
    let result = s.validate_okf(&c, &PageQuery::default()).await.unwrap();
    assert_eq!(result["issues"][0]["path"], "/invalid.md");
}

#[tokio::test]
async fn shared_workspace_knowledge_uses_calling_session_binding() {
    use everruns_contracts::typed_id::{PrincipalId, WorkspaceId};
    use everruns_core::{SessionFileSystem, WorkspaceScopedFileSystem};
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    write(&s, &c, "/guide.md", b"knowledge scoped to this session").await;
    let owner =
        s.db.create_session(crate::storage::CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            ..Default::default()
        })
        .await
        .unwrap();
    let reader =
        s.db.create_session(crate::storage::CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            workspace_id: Some(owner.workspace_id),
            capabilities: json!([{"ref":"knowledge","config":{"collections":[c.public_id()]}}]),
            ..Default::default()
        })
        .await
        .unwrap();
    let fs = WorkspaceScopedFileSystem::wrap(
        Arc::new(everruns_core::host::WriteBlocklistFileStore::new(Arc::new(
            crate::domains::session_files::WorkspaceFileService::new(s.db.clone()),
        ))),
        WorkspaceId::from_uuid(owner.workspace_id),
    );
    let path = format!("/knowledge/{}/guide.md", c.public_id());
    assert!(
        fs.read_file(reader.id, &path)
            .await
            .unwrap()
            .unwrap()
            .content
            .unwrap()
            .contains("scoped to this session")
    );
    assert!(fs.read_file(owner.id, &path).await.unwrap().is_none());
    assert_eq!(
        fs.grep_files(
            reader.id,
            "scoped",
            Some(&format!("/knowledge/{}", c.public_id()))
        )
        .await
        .unwrap()
        .len(),
        1
    );
    use crate::domains::common::Command;
    let ctx = Ctx::minimal_for_test(everruns_core::Caller::internal(1), s.db.clone(), None);
    let result = crate::domains::session_files::commands::GetWorkspaceFile {
        session_id: reader.id.to_string(),
        path: path.clone(),
        recursive: false,
    }
    .run(&ctx)
    .await
    .unwrap();
    assert!(matches!(
        result,
        crate::domains::session_files::types::GetResponse::File(_)
    ));
}

#[tokio::test]
async fn grep_reports_oversized_text_instead_of_false_no_matches() {
    use everruns_contracts::typed_id::PrincipalId;
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    write(&s, &c, "/large.txt", &vec![b'a'; 512 * 1024 + 1]).await;
    let reader =
        s.db.create_session(crate::storage::CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            capabilities: json!([{"ref":"knowledge","config":{"collections":[c.public_id()]}}]),
            ..Default::default()
        })
        .await
        .unwrap();
    let error = s
        .mount_grep(
            reader.id.uuid(),
            "a",
            Some(&format!("/knowledge/{}", c.public_id())),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("scan budget"));
}

#[tokio::test]
#[ignore = "capacity qualification: million-file catalog, run explicitly"]
async fn knowledge_million_file_catalog_qualification() {
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    let source: Uuid =
        sqlx::query_scalar("SELECT id FROM knowledge_sources WHERE collection_id=$1")
            .bind(c.id)
            .fetch_one(s.db.pool())
            .await
            .unwrap();
    let started = std::time::Instant::now();
    sqlx::query("INSERT INTO knowledge_files(id,org_id,collection_id,source_id,path) SELECT gen_random_uuid(),$1,$2,$3,'/docs/'||lpad(n::text,7,'0')||'.md' FROM generate_series(1,1000000) n").bind(c.org_id).bind(c.id).bind(source).execute(s.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO knowledge_versions(id,org_id,file_id,content_hash,size_bytes,mime_type,raw_key,raw_parts,metadata,extraction_status,source_uri) SELECT gen_random_uuid(),org_id,id,'hash',0,'text/markdown','knowledge/test/empty',1,'{}','ready','test://' FROM knowledge_files WHERE collection_id=$1").bind(c.id).execute(s.db.pool()).await.unwrap();
    sqlx::query("UPDATE knowledge_files f SET current_version=v.id FROM knowledge_versions v WHERE v.file_id=f.id AND f.collection_id=$1").bind(c.id).execute(s.db.pool()).await.unwrap();
    eprintln!("million-file catalog loaded in {:?}", started.elapsed());
    for after in ["", "/docs/0500000.md", "/docs/0999500.md"] {
        let started = std::time::Instant::now();
        let page = s
            .files_page(
                &c,
                &PageQuery {
                    after: Some(after.into()),
                    limit: Some(100),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page["data"].as_array().unwrap().len(), 100);
        assert!(serde_json::to_vec(&page).unwrap().len() < 200_000);
        eprintln!(
            "bounded catalog page after {after}: {:?}",
            started.elapsed()
        );
    }
}

#[tokio::test]
async fn streamed_original_memory_is_bounded_and_failed_transfer_cleans_segments() {
    let s = fixture();
    let store = content::Content::new(s.db.clone());
    let chunk = axum::body::Bytes::from(vec![77u8; content::SEGMENT]);
    let stream = futures::stream::iter((0..64).map(|_| Ok::<_, std::io::Error>(chunk.clone())));
    let stored = store.store(1, Uuid::now_v7(), stream).await.unwrap();
    assert_eq!(stored.bytes, 64 * content::SEGMENT as i64);
    assert_eq!(stored.parts, 64);
    assert!(stored.extraction.is_empty());
    assert_eq!(
        store
            .range(
                &stored.key,
                stored.bytes as u64,
                stored.bytes as u64 - 17,
                17
            )
            .await
            .unwrap(),
        vec![77; 17]
    );
    store.remove(&stored.key, stored.parts).await.unwrap();
    let version = Uuid::now_v7();
    let broken = futures::stream::iter([Ok(chunk), Err(std::io::Error::other("interrupted"))]);
    assert!(store.store(1, version, broken).await.is_err());
    assert!(
        store
            .get(&format!("knowledge/org-1/versions/{version}/raw/0"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn management_download_remains_available_after_an_agent_read_deny() {
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    write(&s, &c, "/original.txt", b"retained for curation").await;
    sqlx::query("UPDATE knowledge_files SET readable=false WHERE collection_id=$1")
        .bind(c.id)
        .execute(s.db.pool())
        .await
        .unwrap();
    assert!(
        s.read_bytes(&c, "/original.txt", &PageQuery::default())
            .await
            .is_err()
    );
    assert_eq!(
        s.read_management_bytes(&c, "/original.txt", &PageQuery::default())
            .await
            .unwrap()
            .1,
        b"retained for curation"
    );
}

#[tokio::test]
async fn retried_source_prunes_files_seen_only_before_the_interruption() {
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    let source:Source=sqlx::query_as("INSERT INTO knowledge_sources(id,org_id,collection_id,kind) VALUES($1,$2,$3,'builtin') RETURNING *")
        .bind(Uuid::now_v7()).bind(c.org_id).bind(c.id).fetch_one(s.db.pool()).await.unwrap();
    let job:Job=sqlx::query_as("INSERT INTO knowledge_jobs(id,org_id,collection_id,source_id,kind,revision,status,lease_token,lease_until) VALUES($1,$2,$3,$4,'source',$5,'running',$6,now()+interval '5 minutes') RETURNING *")
        .bind(Uuid::now_v7()).bind(c.org_id).bind(c.id).bind(source.id).bind(c.revision).bind(Uuid::now_v7()).fetch_one(s.db.pool()).await.unwrap();
    // The previous attempt of this same durable job observed this file.
    s.ingest(
        &c,
        &source,
        FileInput {
            path: "/gone.md".into(),
            ..Default::default()
        },
        futures::stream::once(async {
            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"old snapshot"))
        }),
        "test://old".into(),
        None,
        Some(job.id),
        None,
    )
    .await
    .unwrap();
    // The successful retry enumerates the bundled docs, where gone.md is absent.
    s.sync_source(&c, &job).await.unwrap();
    assert!(s.file(&c, "/gone.md").await.is_err());
    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM knowledge_files WHERE source_id=$1 AND deleted_at IS NULL",
    )
    .bind(source.id)
    .fetch_one(s.db.pool())
    .await
    .unwrap();
    assert_eq!(
        remaining,
        everruns_capabilities::knowledge_docs::FILES.len() as i64
    );
}

#[tokio::test]
async fn malformed_or_unknown_read_rules_are_rejected_instead_of_ignored() {
    let s = fixture();
    for rules in [
        json!([]),
        json!({"read_excludes":["private/**"]}),
        json!({"read_exclude":"private/**"}),
    ] {
        assert!(
            s.create(
                1,
                CollectionInput {
                    name: format!("rules-{}", Uuid::now_v7()),
                    rules,
                    ..Default::default()
                }
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn knowledge_mutations_join_the_current_command_transaction() {
    let s = fixture();
    let name = format!("rolled-back-{}", Uuid::now_v7());
    let result: Result<(), CommandError> = crate::storage::transaction::scope(
        &s.db,
        async {
            let created = s
                .create(
                    1,
                    CollectionInput {
                        name: name.clone(),
                        ..Default::default()
                    },
                )
                .await?;
            assert!(
                s.collection(1, created["id"].as_str().unwrap())
                    .await
                    .is_ok()
            );
            Err(CommandError::bad_request("abort composed command"))
        },
        CommandError::internal,
    )
    .await;
    assert!(result.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM knowledge_collections WHERE name=$1")
        .bind(name)
        .fetch_one(s.db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn readonly_callers_can_search_without_gaining_collection_mutation() {
    use crate::domains::common::Command;
    let s = fixture();
    let c = create(&s, IndexMode::Lexical).await;
    write(&s, &c, "/guide.md", b"readonly search evidence").await;
    drain(&s).await;
    let mut caller = everruns_core::Caller::internal(1);
    caller.read_only = true;
    let ctx = Ctx::minimal_for_test(caller, s.db.clone(), None);
    let result = commands::SearchKnowledgeCollections {
        query: KnowledgeQuery {
            query: "readonly".into(),
            collections: vec![c.public_id()],
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(result["results"].as_array().unwrap().len(), 1);
    assert!(
        commands::ArchiveKnowledge {
            collection_id: c.public_id()
        }
        .run(&ctx)
        .await
        .is_err()
    );
}

#[tokio::test]
async fn duplicate_collection_names_are_conflicts() {
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    let error = s
        .create(
            1,
            CollectionInput {
                name: c.name,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
}

#[tokio::test]
async fn agents_can_page_large_unindexed_catalogs_without_disclosing_denied_files() {
    use everruns_contracts::{knowledge::KnowledgeCatalogQuery, typed_id::PrincipalId};
    let s = fixture();
    let c = create(&s, IndexMode::Off).await;
    let source: Uuid =
        sqlx::query_scalar("SELECT id FROM knowledge_sources WHERE collection_id=$1")
            .bind(c.id)
            .fetch_one(s.db.pool())
            .await
            .unwrap();
    sqlx::query("INSERT INTO knowledge_files(id,org_id,collection_id,source_id,path) SELECT gen_random_uuid(),$1,$2,$3,'/docs/'||lpad(n::text,5,'0')||'.md' FROM generate_series(1,1203) n").bind(c.org_id).bind(c.id).bind(source).execute(s.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO knowledge_versions(id,org_id,file_id,content_hash,size_bytes,mime_type,raw_key,raw_parts,metadata,extraction_status,source_uri) SELECT gen_random_uuid(),org_id,id,'hash',0,'text/markdown','knowledge/test/empty',1,'{\"custom\":{\"owner\":\"support\"}}','ready','test://' FROM knowledge_files WHERE collection_id=$1").bind(c.id).execute(s.db.pool()).await.unwrap();
    sqlx::query("UPDATE knowledge_files f SET current_version=v.id FROM knowledge_versions v WHERE v.file_id=f.id AND f.collection_id=$1").bind(c.id).execute(s.db.pool()).await.unwrap();
    sqlx::query("UPDATE knowledge_files SET readable=false WHERE collection_id=$1 AND path='/docs/00051.md'").bind(c.id).execute(s.db.pool()).await.unwrap();
    sqlx::query("UPDATE knowledge_collections SET rules=$2 WHERE id=$1")
        .bind(c.id)
        .bind(json!({"read_exclude":["docs/00050.md"]}))
        .execute(s.db.pool())
        .await
        .unwrap();
    let session =
        s.db.create_session(crate::storage::CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            capabilities: json!([{"ref":"knowledge","config":{"collections":[c.public_id()]}}]),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut after = None;
    let mut paths = std::collections::BTreeSet::new();
    loop {
        let page = s
            .session_catalog(
                1,
                session.id,
                KnowledgeCatalogQuery {
                    collection: c.public_id(),
                    path: Some("/docs/".into()),
                    after,
                    limit: 100,
                    include_metadata: false,
                },
            )
            .await
            .unwrap();
        assert!(page.files.len() <= 100);
        for file in page.files {
            assert!(!file.path.ends_with("00050.md") && !file.path.ends_with("00051.md"));
            assert!(file.metadata.is_none());
            assert!(paths.insert(file.path));
        }
        after = page.next_cursor;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(paths.len(), 1201);
    let page = s
        .session_catalog(
            1,
            session.id,
            KnowledgeCatalogQuery {
                collection: c.public_id(),
                path: None,
                after: None,
                limit: 100,
                include_metadata: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.files.len(), 10);
    assert_eq!(
        page.files[0].metadata.as_ref().unwrap()["custom"]["owner"],
        "support"
    );
    let other =
        s.db.create_session(crate::storage::CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        s.session_catalog(
            1,
            other.id,
            KnowledgeCatalogQuery {
                collection: c.public_id(),
                path: None,
                after: None,
                limit: 50,
                include_metadata: false
            }
        )
        .await
        .is_err()
    );
    let index = s
        .mount_read(
            session.id.uuid(),
            &format!("/knowledge/{}/index.md", c.public_id()),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(index.content.unwrap().contains("browse_knowledge"));
}

#[tokio::test]
async fn collection_history_restores_configuration_without_content_or_worker_noise() {
    use crate::domains::common::{Ctx, dispatch};
    let s = fixture();
    let ctx = Ctx::minimal_for_test(everruns_core::Caller::internal(1), s.db.clone(), None);
    async fn command(ctx: &Ctx, name: &str, params: serde_json::Value) -> serde_json::Value {
        serde_json::from_str(&dispatch(name, params, ctx).await.unwrap()).unwrap()
    }
    let created = command(
        &ctx,
        "create_knowledge",
        json!({
            "name": "Audited handbook", "description": "Original configuration", "indexing": "off"
        }),
    )
    .await;
    let id = created["id"].as_str().unwrap();
    let c = s.collection(1, id).await.unwrap();
    write(
        &s,
        &c,
        "/private.bin",
        b"binary contents must stay out of configuration history",
    )
    .await;
    command(&ctx, "update_knowledge", json!({
        "collection_id": id, "name": "Audited handbook", "description": "Original configuration", "indexing": "off"
    })).await;
    let entries = command(&ctx, "list_entity_history", json!({"entity_ref": id})).await;
    assert_eq!(
        entries.as_array().unwrap().len(),
        1,
        "content and no-op configuration do not create revisions: {entries}"
    );
    command(&ctx, "update_knowledge", json!({
        "collection_id": id, "name": "Audited handbook", "description": "Changed configuration", "indexing": "lexical"
    })).await;
    drain(&s).await;
    command(&ctx, "update_knowledge", json!({
        "collection_id": id, "name": "Audited handbook", "description": "Changed configuration", "indexing": "lexical"
    })).await;
    let entries = command(&ctx, "list_entity_history", json!({"entity_ref": id})).await;
    assert_eq!(
        entries.as_array().unwrap().len(),
        2,
        "indexing progress does not change the configuration snapshot: {entries}"
    );
    command(
        &ctx,
        "restore_entity_revision",
        json!({"entity_ref": id, "revision": 1}),
    )
    .await;
    let restored = s.collection(1, id).await.unwrap();
    assert_eq!(restored.description, "Original configuration");
    assert_eq!(restored.indexing, "off");
    let (_, bytes) = s
        .read_bytes(&restored, "/private.bin", &PageQuery::default())
        .await
        .unwrap();
    assert_eq!(
        bytes,
        b"binary contents must stay out of configuration history"
    );
    let entries = command(&ctx, "list_entity_history", json!({"entity_ref": id})).await;
    assert_eq!(entries[0]["action"], "restored");
    assert_eq!(entries[0]["restored_from_revision"], 1);
}
