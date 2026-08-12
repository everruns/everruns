// Tests for the Knowledge Index Syncout pipeline. Live network coverage is ignored by default.

use super::*;
use crate::services::ProviderResolverService;
use crate::storage::StorageBackend;
use crate::storage::encryption::{EncryptionService, generate_encryption_key};
use crate::storage::models::{CreateKnowledgeIndexRow, CreateModelRow, CreateProviderRow};
use async_trait::async_trait;
use everruns_core::CredentialFormSchema;
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::driver_registry::{
    BoxedEmbeddingsDriver, DriverDescriptor, DriverId, EmbedResponse, EmbeddingsDriver,
    EmbeddingsDriverError, ServiceKind,
};
use everruns_core::typed_id::KnowledgeIndexId;
use everruns_platform::vector_store::{InMemoryVectorStore, VectorQuery, index_namespace};

// ----------------------------- chunk_text -----------------------------

#[test]
fn chunk_text_short_input_single_chunk() {
    let chunks = chunk_text("hello world", 1500, 150);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].text, "hello world");
    assert_eq!(chunks[0].char_start, 0);
    assert_eq!(chunks[0].char_end, 11);
}

#[test]
fn chunk_text_empty_and_whitespace_yield_nothing() {
    assert!(chunk_text("", 100, 10).is_empty());
    assert!(chunk_text("   \n\t ", 100, 10).is_empty());
}

#[test]
fn chunk_text_windows_with_overlap_and_offsets() {
    // 25 chars, size 10, overlap 4 => step 6: [0,10), [6,16), [12,22), [18,25)
    let text: String = ('a'..='y').collect(); // 25 chars
    let chunks = chunk_text(&text, 10, 4);
    let bounds: Vec<(usize, usize)> = chunks.iter().map(|c| (c.char_start, c.char_end)).collect();
    assert_eq!(bounds, vec![(0, 10), (6, 16), (12, 22), (18, 25)]);
    // Overlap between adjacent chunks is preserved.
    assert_eq!(&text[6..10], &chunks[0].text[6..10]);
    // Reconstructable: the last chunk ends at the document end.
    assert_eq!(chunks.last().unwrap().char_end, 25);
}

#[test]
fn chunk_text_overlap_clamped_below_size() {
    // overlap >= size must still make forward progress (no infinite loop).
    let text: String = std::iter::repeat_n('x', 30).collect();
    let chunks = chunk_text(&text, 10, 50);
    assert!(chunks.len() >= 3);
    assert_eq!(chunks.last().unwrap().char_end, 30);
}

#[test]
fn chunk_text_counts_unicode_scalars_not_bytes() {
    // Each emoji is one scalar but multiple bytes; offsets are scalar-based.
    let text = "😀😀😀😀😀"; // 5 scalars
    let chunks = chunk_text(text, 2, 0);
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].char_start, 0);
    assert_eq!(chunks[0].char_end, 2);
    assert_eq!(chunks[2].char_end, 5);
}

#[test]
fn stored_github_url_resolves_to_canonical_anonymous_clone() {
    let source = ResolvedGitSource::from_github(
        GithubSourceConfig {
            repository: "https://github.com/everruns/bashkit/".into(),
            branch: "main".into(),
            root_folder: Some("knowledge".into()),
        },
        None,
    )
    .expect("resolve source");

    assert_eq!(source.repository, "everruns/bashkit");
    assert_eq!(source.url, "https://github.com/everruns/bashkit.git");
    assert!(source.auth_token.is_none());
}

#[test]
#[ignore = "requires anonymous network access to github.com"]
fn live_stored_github_url_retry_reads_bashkit_knowledge_root() {
    let source = ResolvedGitSource::from_github(
        GithubSourceConfig {
            repository: "https://github.com/everruns/bashkit/".into(),
            branch: "main".into(),
            root_folder: Some("knowledge".into()),
        },
        None,
    )
    .expect("resolve historical source");
    let documents = snapshot_documents(
        source,
        KnowledgeIndexSyncConfig {
            poll_interval: Duration::ZERO,
            max_files: 5_000,
            max_file_bytes: 5 * 1024 * 1024,
            max_total_bytes: 100 * 1024 * 1024,
        },
    )
    .expect("clone and read knowledge root");

    assert!(!documents.is_empty());
    assert!(documents.iter().all(|document| {
        document
            .source_uri
            .starts_with("github://everruns/bashkit@main/")
    }));
}

// ------------------------- sync claim/complete/fail -------------------------

async fn seed_index(db: &StorageBackend, org_id: i64, model_id: everruns_core::ModelId) -> Uuid {
    let public_id = KnowledgeIndexId::new().to_string();
    let vector_namespace = index_namespace(org_id, &public_id);
    let row = db
        .create_knowledge_index(
            org_id,
            CreateKnowledgeIndexRow {
                public_id,
                name: format!("Idx-{}", Uuid::now_v7()),
                description: None,
                source_type: "github".to_string(),
                source_config: serde_json::json!({"repository": "owner/repo", "branch": "main"}),
                embedding_model_id: model_id,
                vector_namespace,
                owner_principal_id: None,
                resolved_owner_user_id: None,
            },
        )
        .await
        .expect("create index");
    row.id
}

async fn seed_embedding_model(
    db: &StorageBackend,
    encryption: &EncryptionService,
    org_id: i64,
) -> everruns_core::ModelId {
    let encrypted = encryption.encrypt_string("test-key").expect("encrypt");
    let provider = db
        .create_provider(
            org_id,
            CreateProviderRow {
                name: "test-embeddings".into(),
                provider_type: "test-embeddings".into(),
                base_url: None,
                api_key_encrypted: Some(encrypted),
                settings: None,
            },
        )
        .await
        .expect("create provider");
    let model = db
        .create_model(
            org_id,
            CreateModelRow {
                provider_id: provider.id,
                model_id: "test-embed-model".into(),
                display_name: "Test Embeddings".into(),
                capabilities: vec!["embeddings".into()],
                enabled: true,
                is_favorite: false,
                source: "manual".into(),
                provider_metadata: None,
            },
        )
        .await
        .expect("create model");
    model.id
}

fn one_doc(name: &str, text: &str) -> ExtractedDocument {
    ExtractedDocument {
        source_uri: format!("github://owner/repo@main/{name}"),
        title: Some(name.to_string()),
        mime_type: Some("text/markdown".to_string()),
        content_hash: hex::encode(Sha256::digest(text.as_bytes())),
        size_bytes: text.len() as u64,
        text: text.to_string(),
    }
}

#[tokio::test]
async fn enqueue_claim_complete_round_trip() {
    let db = Arc::new(StorageBackend::in_memory());
    let encryption = EncryptionService::new(&generate_encryption_key("kek-v1"), &[]).unwrap();
    let model_id = seed_embedding_model(&db, &encryption, DEFAULT_ORG_ID).await;
    let index_id = seed_index(&db, DEFAULT_ORG_ID, model_id).await;

    // Creation is already pending; an explicit retry remains idempotent.
    db.enqueue_knowledge_index_sync(DEFAULT_ORG_ID, index_id)
        .await
        .expect("enqueue")
        .expect("row");

    let claimed = db
        .claim_next_knowledge_index_sync()
        .await
        .expect("claim")
        .expect("pending index");
    assert_eq!(claimed.id, index_id);
    assert_eq!(claimed.sync_status, "syncing");

    let docs = vec![CreateKnowledgeIndexDocumentWithChunks {
        public_id: KnowledgeIndexDocumentId::new().to_string(),
        source_uri: "github://owner/repo@main/a.md".into(),
        title: Some("a.md".into()),
        mime_type: Some("text/markdown".into()),
        content_hash: Some("hash".into()),
        size_bytes: Some(5),
        chunks: vec![CreateKnowledgeIndexChunkRow {
            public_id: KnowledgeIndexChunkId::new().to_string(),
            ordinal: 0,
            text: "hello".into(),
            location: Some(serde_json::json!({"char_start": 0, "char_end": 5})),
            token_count: Some(2),
        }],
    }];

    db.complete_knowledge_index_sync(index_id, claimed.updated_at, docs, Some(3))
        .await
        .expect("complete")
        .expect("synced");

    let stored = db
        .get_knowledge_index_by_id(DEFAULT_ORG_ID, index_id)
        .await
        .expect("get")
        .expect("row");
    assert_eq!(stored.sync_status, "synced");
    assert_eq!(stored.vector_dim, Some(3));
    assert!(stored.last_synced_at.is_some());

    let documents = db
        .list_knowledge_index_documents(index_id)
        .await
        .expect("docs");
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].chunk_count, 1);
    let chunks = db
        .list_knowledge_index_chunks(index_id)
        .await
        .expect("chunks");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].text, "hello");
}

#[tokio::test]
async fn complete_rejects_stale_claim() {
    let db = Arc::new(StorageBackend::in_memory());
    let encryption = EncryptionService::new(&generate_encryption_key("kek-v1"), &[]).unwrap();
    let model_id = seed_embedding_model(&db, &encryption, DEFAULT_ORG_ID).await;
    let index_id = seed_index(&db, DEFAULT_ORG_ID, model_id).await;

    let created = db
        .get_knowledge_index_by_id(DEFAULT_ORG_ID, index_id)
        .await
        .expect("get created index")
        .expect("created index");
    assert_eq!(created.sync_status, "pending");

    db.enqueue_knowledge_index_sync(DEFAULT_ORG_ID, index_id)
        .await
        .expect("enqueue");
    let claimed = db
        .claim_next_knowledge_index_sync()
        .await
        .expect("claim")
        .expect("pending");

    let stale = claimed.updated_at - chrono::Duration::seconds(1);
    let result = db
        .complete_knowledge_index_sync(index_id, stale, vec![], Some(3))
        .await
        .expect("complete");
    assert!(result.is_none(), "stale claim must not complete");

    let stored = db
        .get_knowledge_index_by_id(DEFAULT_ORG_ID, index_id)
        .await
        .expect("get")
        .expect("row");
    assert_eq!(stored.sync_status, "syncing");
    assert!(
        db.list_knowledge_index_documents(index_id)
            .await
            .expect("docs")
            .is_empty()
    );
}

#[tokio::test]
async fn fail_rejects_stale_claim_and_preserves_status() {
    let db = Arc::new(StorageBackend::in_memory());
    let encryption = EncryptionService::new(&generate_encryption_key("kek-v1"), &[]).unwrap();
    let model_id = seed_embedding_model(&db, &encryption, DEFAULT_ORG_ID).await;
    let index_id = seed_index(&db, DEFAULT_ORG_ID, model_id).await;

    db.enqueue_knowledge_index_sync(DEFAULT_ORG_ID, index_id)
        .await
        .expect("enqueue");
    let claimed = db
        .claim_next_knowledge_index_sync()
        .await
        .expect("claim")
        .expect("pending");

    let stale = claimed.updated_at - chrono::Duration::seconds(1);
    assert!(
        db.fail_knowledge_index_sync(index_id, stale, "boom")
            .await
            .expect("fail")
            .is_none()
    );

    // Valid claim fails correctly.
    let failed = db
        .fail_knowledge_index_sync(index_id, claimed.updated_at, "boom")
        .await
        .expect("fail")
        .expect("row");
    assert_eq!(failed.sync_status, "failed");
    assert_eq!(failed.last_sync_error.as_deref(), Some("boom"));
}

// ------------------------- end-to-end embed + vectors -------------------------

/// Deterministic test embeddings driver: maps text length and first byte to a
/// fixed 3-dim vector so assertions are stable without any network call.
struct DeterministicEmbeddingsDriver;

#[async_trait]
impl EmbeddingsDriver for DeterministicEmbeddingsDriver {
    async fn embed(
        &self,
        _endpoint: &everruns_core::ProviderEndpoint,
        request: EmbedRequest,
    ) -> std::result::Result<EmbedResponse, EmbeddingsDriverError> {
        let embeddings = request
            .texts
            .iter()
            .map(|text| {
                let first = text.bytes().next().unwrap_or(0) as f32;
                let len = text.chars().count() as f32;
                vec![first, len, 1.0]
            })
            .collect();
        Ok(EmbedResponse {
            embeddings,
            usage_tokens: Some(0),
        })
    }
}

fn test_driver_registry() -> Arc<DriverRegistry> {
    let mut registry = DriverRegistry::new();
    registry.register_descriptor(DriverDescriptor {
        id: DriverId::external("test-embeddings"),
        display_name: "Test Embeddings".into(),
        services: vec![ServiceKind::Embeddings],
        credential_schema: CredentialFormSchema::empty(),
        oauth: None,
        chat: None,
        embeddings: Some(Arc::new(|_config| {
            Box::new(DeterministicEmbeddingsDriver) as BoxedEmbeddingsDriver
        })),
    });
    Arc::new(registry)
}

#[tokio::test]
async fn embed_persists_documents_chunks_and_vectors() {
    let db = Arc::new(StorageBackend::in_memory());
    let encryption =
        Arc::new(EncryptionService::new(&generate_encryption_key("kek-v1"), &[]).unwrap());
    let model_id = seed_embedding_model(&db, &encryption, DEFAULT_ORG_ID).await;
    let index_id = seed_index(&db, DEFAULT_ORG_ID, model_id).await;
    let created = db
        .get_knowledge_index_by_id(DEFAULT_ORG_ID, index_id)
        .await
        .expect("get created index")
        .expect("created index");
    assert_eq!(created.sync_status, "pending");

    let driver_registry = test_driver_registry();
    let provider_resolver = Arc::new(
        ProviderResolverService::new(db.clone(), Some(encryption.clone()))
            .with_driver_registry((*driver_registry).clone()),
    );
    let vector_store: Arc<dyn VectorStore> = Arc::new(InMemoryVectorStore::new());

    let service = KnowledgeIndexSyncService::new(
        db.clone(),
        None,
        provider_resolver,
        driver_registry,
        vector_store.clone(),
        KnowledgeIndexSyncConfig::from_env(),
    );

    // Claim the automatically enqueued index, then drive the embed + persist
    // path directly (the git checkout is exercised separately below).
    let claimed = db
        .claim_next_knowledge_index_sync()
        .await
        .expect("claim")
        .expect("pending");
    assert_eq!(claimed.sync_status, "syncing");

    let long_text: String = std::iter::repeat_n('a', 3200).collect();
    let docs = vec![
        one_doc("guide.md", &long_text),
        one_doc("intro.md", "short body"),
    ];

    let prepared = service
        .embed_documents(&claimed, docs)
        .await
        .expect("embed");
    assert_eq!(prepared.vector_dim, Some(3));
    // 3200 chars / (1500 - 150 step) windows => 3 chunks for the long doc, 1 short.
    let total_chunks: usize = prepared.documents.iter().map(|d| d.chunks.len()).sum();
    assert!(
        total_chunks >= 4,
        "expected multiple chunks, got {total_chunks}"
    );

    let outcome = service
        .persist(claimed.clone(), claimed.updated_at, prepared)
        .await
        .expect("persist")
        .expect("outcome");
    match outcome {
        KnowledgeIndexSyncOutcome::Synced {
            document_count,
            chunk_count,
            ..
        } => {
            assert_eq!(document_count, 2);
            assert!(chunk_count >= 4);
        }
        other => panic!("expected synced, got {other:?}"),
    }

    // Postgres-side state.
    let stored = db
        .get_knowledge_index_by_id(DEFAULT_ORG_ID, index_id)
        .await
        .expect("get")
        .expect("row");
    assert_eq!(stored.sync_status, "synced");
    assert_eq!(stored.vector_dim, Some(3));
    let documents = db
        .list_knowledge_index_documents(index_id)
        .await
        .expect("docs");
    assert_eq!(documents.len(), 2);
    let document_counts = db
        .count_knowledge_index_documents(&[index_id])
        .await
        .expect("document counts");
    assert_eq!(document_counts.get(&index_id), Some(&2));
    let chunks = db
        .list_knowledge_index_chunks(index_id)
        .await
        .expect("chunks");
    assert!(chunks.len() >= 4);

    // Vector store namespace populated and queryable.
    let namespace = stored.vector_namespace.expect("namespace");
    let matches = vector_store
        .query(
            &namespace,
            VectorQuery {
                vector: Some(vec![97.0, 10.0, 1.0]),
                text: None,
                top_k: 10,
            },
        )
        .await
        .expect("query");
    assert!(
        !matches.is_empty(),
        "vector store namespace should be populated"
    );
    // Every returned chunk id corresponds to a persisted chunk public_id.
    let chunk_ids: std::collections::HashSet<_> =
        chunks.iter().map(|c| c.public_id.clone()).collect();
    for m in &matches {
        assert!(
            chunk_ids.contains(&m.id),
            "match {} not in persisted chunks",
            m.id
        );
    }
}

#[test]
fn github_root_folder_limits_ingestion_and_keeps_relative_source_uris() {
    let checkout = tempfile::TempDir::new().expect("checkout");
    let knowledge = checkout.path().join("knowledge");
    std::fs::create_dir_all(knowledge.join("guides")).expect("create root folder");
    std::fs::write(checkout.path().join("outside.md"), "outside").expect("write outside");
    std::fs::write(knowledge.join("guides/start.md"), "inside").expect("write inside");

    let root = resolve_root_folder(checkout.path(), Some("knowledge")).expect("resolve root");
    let source = ResolvedGitSource {
        url: "https://github.com/everruns/bashkit.git".into(),
        repository: "everruns/bashkit".into(),
        branch: "main".into(),
        root_folder: Some("knowledge".into()),
        auth_token: None,
    };
    let limits = SnapshotLimits {
        max_files: 10,
        max_file_bytes: 1024,
        max_total_bytes: 4096,
    };
    let mut documents = Vec::new();
    let mut total_bytes = 0;
    collect_documents(
        &root,
        &root,
        &source,
        &limits,
        &mut documents,
        &mut total_bytes,
    )
    .expect("collect documents");

    assert_eq!(documents.len(), 1);
    assert_eq!(
        documents[0].source_uri,
        "github://everruns/bashkit@main/guides/start.md"
    );
}
