// End-to-end tests for Knowledge Index retrieval (`search_index`).
// Offline: deterministic test embeddings driver + in-memory vector store.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::DEFAULT_ORG_ID;
use everruns_platform::vector_store::{
    InMemoryVectorStore, KnowledgeIndexSearch, VectorRecord, VectorStore, index_namespace,
};
use everruns_provider::credential_schema::CredentialFormSchema;
use everruns_provider::driver_registry::{
    BoxedEmbeddingsDriver, DriverDescriptor, DriverId, DriverRegistry, EmbedRequest, EmbedResponse,
    EmbeddingsDriver, EmbeddingsDriverError, ServiceKind,
};
use everruns_provider::typed_id::{
    KnowledgeIndexChunkId, KnowledgeIndexDocumentId, KnowledgeIndexId,
};
use uuid::Uuid;

use super::KnowledgeIndexSearchService;
use crate::services::ProviderResolverService;
use crate::storage::StorageBackend;
use crate::storage::encryption::{EncryptionService, generate_encryption_key};
use crate::storage::models::{
    CreateKnowledgeIndexChunkRow, CreateKnowledgeIndexDocumentWithChunks, CreateKnowledgeIndexRow,
    CreateModelRow, CreateProviderRow,
};

/// Deterministic embeddings driver: maps text to a stable 3-dim vector so that
/// ranking is reproducible without any network call. The first dimension is the
/// first byte, which lets us steer which chunk a query ranks highest.
struct DeterministicEmbeddingsDriver;

#[async_trait]
impl EmbeddingsDriver for DeterministicEmbeddingsDriver {
    async fn embed(
        &self,
        _endpoint: &everruns_provider::runtime_provider::ProviderEndpoint,
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
        // Bill one token per input character and a fixed per-call cost, so
        // usage assertions distinguish "reported" from "defaulted to zero".
        let billed: u32 = request.texts.iter().map(|t| t.chars().count() as u32).sum();
        Ok(EmbedResponse {
            embeddings,
            usage_tokens: Some(billed),
            actual_cost_usd: Some(0.0001),
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

async fn seed_embedding_model(
    db: &StorageBackend,
    encryption: &EncryptionService,
    org_id: i64,
) -> everruns_provider::typed_id::ModelId {
    let encrypted = encryption.encrypt_string("test-key").expect("encrypt");
    let provider = db
        .create_provider(
            org_id,
            CreateProviderRow {
                name: format!("test-embeddings-{}", Uuid::now_v7()),
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

/// Seed an index and synchronously populate documents + chunks + vectors.
/// Returns the index public_id and its namespace.
async fn seed_populated_index(
    db: &StorageBackend,
    vector_store: &Arc<dyn VectorStore>,
    org_id: i64,
    model_id: everruns_provider::typed_id::ModelId,
    chunks: &[(&str, &str)], // (source_file, chunk_text)
) -> (String, String) {
    let public_id = KnowledgeIndexId::new().to_string();
    let namespace = index_namespace(org_id, &public_id);
    let index = db
        .create_knowledge_index(
            org_id,
            CreateKnowledgeIndexRow {
                public_id: public_id.clone(),
                name: format!("Idx-{}", Uuid::now_v7()),
                description: None,
                source_type: "github".to_string(),
                source_config: serde_json::json!({"repository": "owner/repo", "branch": "main"}),
                embedding_model_id: model_id,
                vector_namespace: namespace.clone(),
                owner_principal_id: None,
                resolved_owner_user_id: None,
            },
        )
        .await
        .expect("create index");

    db.enqueue_knowledge_index_sync(org_id, index.id)
        .await
        .expect("enqueue");
    let claimed = db
        .claim_next_knowledge_index_sync()
        .await
        .expect("claim")
        .expect("pending");

    let mut documents = Vec::new();
    let mut records = Vec::new();
    for (file, text) in chunks {
        let doc_public_id = KnowledgeIndexDocumentId::new().to_string();
        let chunk_public_id = KnowledgeIndexChunkId::new().to_string();
        records.push(VectorRecord {
            id: chunk_public_id.clone(),
            vector: vec![
                text.bytes().next().unwrap_or(0) as f32,
                text.chars().count() as f32,
                1.0,
            ],
            text: text.to_string(),
            document_id: doc_public_id.clone(),
        });
        documents.push(CreateKnowledgeIndexDocumentWithChunks {
            public_id: doc_public_id,
            source_uri: format!("github://owner/repo@main/{file}"),
            title: Some((*file).to_string()),
            mime_type: Some("text/markdown".into()),
            content_hash: Some("hash".into()),
            size_bytes: Some(text.len() as i64),
            chunks: vec![CreateKnowledgeIndexChunkRow {
                public_id: chunk_public_id,
                ordinal: 0,
                text: (*text).to_string(),
                location: Some(
                    serde_json::json!({"char_start": 0, "char_end": text.chars().count()}),
                ),
                token_count: Some(1),
            }],
        });
    }

    db.complete_knowledge_index_sync(index.id, claimed.updated_at, documents, Some(3))
        .await
        .expect("complete")
        .expect("synced");
    vector_store
        .upsert(&namespace, records)
        .await
        .expect("upsert vectors");

    (public_id, namespace)
}

fn build_service(
    db: Arc<StorageBackend>,
    encryption: Arc<EncryptionService>,
    vector_store: Arc<dyn VectorStore>,
) -> KnowledgeIndexSearchService {
    let driver_registry = test_driver_registry();
    let provider_resolver = Arc::new(
        ProviderResolverService::new(db.clone(), Some(encryption))
            .with_driver_registry((*driver_registry).clone()),
    );
    KnowledgeIndexSearchService::new(db, provider_resolver, driver_registry, vector_store)
}

#[tokio::test]
async fn search_returns_ordered_citations() {
    let db = Arc::new(StorageBackend::in_memory());
    let encryption =
        Arc::new(EncryptionService::new(&generate_encryption_key("kek-v1"), &[]).unwrap());
    let model_id = seed_embedding_model(&db, &encryption, DEFAULT_ORG_ID).await;
    let vector_store: Arc<dyn VectorStore> = Arc::new(InMemoryVectorStore::new());

    // Two chunks whose first byte differs so the query vector ranks one first.
    let (index_id, _ns) = seed_populated_index(
        &db,
        &vector_store,
        DEFAULT_ORG_ID,
        model_id,
        &[
            ("alpha.md", "alpha database indexing guide"),
            ("beta.md", "beta cooking recipes"),
        ],
    )
    .await;

    let service = build_service(db.clone(), encryption.clone(), vector_store.clone());

    // Query text starting with 'a' steers cosine toward the "alpha" chunk.
    let citations = service
        .search(
            DEFAULT_ORG_ID,
            std::slice::from_ref(&index_id),
            "alpha indexing",
            10,
        )
        .await
        .expect("search")
        .citations;

    assert_eq!(citations.len(), 2, "both chunks should be returned");
    // Highest score first.
    assert!(citations[0].score >= citations[1].score);
    let top = &citations[0];
    assert_eq!(top.index_id, index_id);
    assert!(top.id.starts_with("kchk_"));
    assert_eq!(top.source_uri, "github://owner/repo@main/alpha.md");
    assert_eq!(top.document_title.as_deref(), Some("alpha.md"));
    assert_eq!(top.snippet, "alpha database indexing guide");
    assert!(top.location.is_some());
}

#[tokio::test]
async fn search_excludes_archived_cross_org_and_unknown_without_error() {
    let db = Arc::new(StorageBackend::in_memory());
    let encryption =
        Arc::new(EncryptionService::new(&generate_encryption_key("kek-v1"), &[]).unwrap());
    let vector_store: Arc<dyn VectorStore> = Arc::new(InMemoryVectorStore::new());

    let org_a = DEFAULT_ORG_ID;
    let org_b = DEFAULT_ORG_ID + 1;
    let model_a = seed_embedding_model(&db, &encryption, org_a).await;
    let model_b = seed_embedding_model(&db, &encryption, org_b).await;

    // Live index in org A (should be searched).
    let (live_id, _) = seed_populated_index(
        &db,
        &vector_store,
        org_a,
        model_a,
        &[("live.md", "alpha live content")],
    )
    .await;

    // Archived index in org A (must be excluded).
    let (archived_id, _) = seed_populated_index(
        &db,
        &vector_store,
        org_a,
        model_a,
        &[("arch.md", "alpha archived")],
    )
    .await;
    let archived = db
        .get_knowledge_index_by_public_id(org_a, &archived_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        db.archive_knowledge_index(org_a, archived.id)
            .await
            .unwrap()
    );

    // Index belonging to org B (must be excluded when queried as org A).
    let (cross_org_id, _) = seed_populated_index(
        &db,
        &vector_store,
        org_b,
        model_b,
        &[("other.md", "alpha cross org")],
    )
    .await;

    let unknown_id = KnowledgeIndexId::new().to_string();

    let service = build_service(db.clone(), encryption.clone(), vector_store.clone());

    let citations = service
        .search(
            org_a,
            &[
                live_id.clone(),
                archived_id.clone(),
                cross_org_id.clone(),
                unknown_id.clone(),
            ],
            "alpha",
            10,
        )
        .await
        .expect("search must not error on excluded ids")
        .citations;

    // Only the live org-A index contributes results.
    assert_eq!(citations.len(), 1);
    assert_eq!(citations[0].index_id, live_id);
    assert_eq!(citations[0].source_uri, "github://owner/repo@main/live.md");
}

#[tokio::test]
async fn search_respects_top_k_across_indexes() {
    let db = Arc::new(StorageBackend::in_memory());
    let encryption =
        Arc::new(EncryptionService::new(&generate_encryption_key("kek-v1"), &[]).unwrap());
    let model_id = seed_embedding_model(&db, &encryption, DEFAULT_ORG_ID).await;
    let vector_store: Arc<dyn VectorStore> = Arc::new(InMemoryVectorStore::new());

    let (idx1, _) = seed_populated_index(
        &db,
        &vector_store,
        DEFAULT_ORG_ID,
        model_id,
        &[("a.md", "alpha one"), ("b.md", "alpha two")],
    )
    .await;
    let (idx2, _) = seed_populated_index(
        &db,
        &vector_store,
        DEFAULT_ORG_ID,
        model_id,
        &[("c.md", "alpha three")],
    )
    .await;

    let service = build_service(db.clone(), encryption.clone(), vector_store.clone());
    let citations = service
        .search(DEFAULT_ORG_ID, &[idx1, idx2], "alpha", 2)
        .await
        .expect("search")
        .citations;
    assert_eq!(citations.len(), 2, "global top_k must cap merged results");
}

#[tokio::test]
async fn search_reports_embedding_usage_per_index() {
    let db = Arc::new(StorageBackend::in_memory());
    let encryption =
        Arc::new(EncryptionService::new(&generate_encryption_key("kek-v1"), &[]).unwrap());
    let model_id = seed_embedding_model(&db, &encryption, DEFAULT_ORG_ID).await;
    let vector_store: Arc<dyn VectorStore> = Arc::new(InMemoryVectorStore::new());

    let (idx1, _) = seed_populated_index(
        &db,
        &vector_store,
        DEFAULT_ORG_ID,
        model_id,
        &[("a.md", "alpha one")],
    )
    .await;
    let (idx2, _) = seed_populated_index(
        &db,
        &vector_store,
        DEFAULT_ORG_ID,
        model_id,
        &[("c.md", "alpha three")],
    )
    .await;

    let service = build_service(db.clone(), encryption.clone(), vector_store.clone());
    let outcome = service
        .search(DEFAULT_ORG_ID, &[idx1, idx2], "alpha", 10)
        .await
        .expect("search");

    // Retrieval embeds the query once per bound index, so each call is
    // reported separately — they may use different embedding models.
    assert_eq!(
        outcome.embedding_usage.len(),
        2,
        "one usage record per embedding call"
    );
    for usage in &outcome.embedding_usage {
        assert_eq!(usage.total_tokens, "alpha".chars().count() as u32);
        assert_eq!(usage.actual_cost_usd, Some(0.0001));
        assert!(
            usage.provider.is_some(),
            "usage must name the provider that served the call"
        );
    }
}

#[tokio::test]
async fn search_reports_embedding_usage_when_nothing_matches() {
    let db = Arc::new(StorageBackend::in_memory());
    let encryption =
        Arc::new(EncryptionService::new(&generate_encryption_key("kek-v1"), &[]).unwrap());
    let model_id = seed_embedding_model(&db, &encryption, DEFAULT_ORG_ID).await;
    let vector_store: Arc<dyn VectorStore> = Arc::new(InMemoryVectorStore::new());

    // An index with no chunks: the query is still embedded and billed, but the
    // vector query matches nothing. The spend must not vanish with the results.
    let (empty_id, _) =
        seed_populated_index(&db, &vector_store, DEFAULT_ORG_ID, model_id, &[]).await;

    let service = build_service(db.clone(), encryption.clone(), vector_store.clone());
    let outcome = service
        .search(DEFAULT_ORG_ID, &[empty_id], "alpha", 10)
        .await
        .expect("search");

    assert!(outcome.citations.is_empty(), "no chunks to match");
    assert_eq!(
        outcome.embedding_usage.len(),
        1,
        "embedding was billed even though retrieval returned nothing"
    );
    assert_eq!(outcome.embedding_usage[0].actual_cost_usd, Some(0.0001));
}
