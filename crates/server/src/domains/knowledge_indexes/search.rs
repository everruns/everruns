//! Knowledge Index retrieval — the server-side `KnowledgeIndexSearch` impl
//! backing the agent `search_index` tool. See knowledge/runtime-resources/knowledge-indexes.md
//! ("Retrieval and citations").
//!
//! Flow: for each bound `kidx_` id, load it org-scoped (silently skipping ids
//! that are missing, cross-org, archived, or deleted — no existence leak),
//! embed the query with that index's own embedding model, run a hybrid query
//! (vector KNN + BM25) against the index's org-derived vector-store namespace,
//! merge matches across indexes by score and take the global `top_k`, then
//! hydrate chunk text + citation metadata from Postgres.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use everruns_core::driver_registry::{DriverRegistry, EmbedRequest};
use everruns_platform::vector_store::{
    KnowledgeIndexCitation, KnowledgeIndexSearch, VectorQuery, VectorStore,
};

use super::embedding::build_embeddings_driver;
use crate::services::ProviderResolverService;
use crate::storage::StorageBackend;
use crate::storage::models::KnowledgeIndexRow;

/// Maximum characters of chunk text returned as a citation snippet.
const SNIPPET_MAX_CHARS: usize = 600;

/// Server-side hybrid retrieval over an org's bound Knowledge Indexes.
#[derive(Clone)]
pub struct KnowledgeIndexSearchService {
    db: Arc<StorageBackend>,
    provider_resolver: Arc<ProviderResolverService>,
    driver_registry: Arc<DriverRegistry>,
    vector_store: Arc<dyn VectorStore>,
}

impl KnowledgeIndexSearchService {
    pub fn new(
        db: Arc<StorageBackend>,
        provider_resolver: Arc<ProviderResolverService>,
        driver_registry: Arc<DriverRegistry>,
        vector_store: Arc<dyn VectorStore>,
    ) -> Self {
        Self {
            db,
            provider_resolver,
            driver_registry,
            vector_store,
        }
    }

    /// Embed `query` with `index`'s embedding model and run a hybrid query
    /// against its namespace, returning `(chunk_public_id, score)` matches.
    ///
    /// Bound indexes may use different embedding models, which makes a single
    /// shared query embedding ambiguous; we embed per index with that index's
    /// own model (the simplest correct behavior).
    async fn query_index(
        &self,
        index: &KnowledgeIndexRow,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<(String, f32)>> {
        let Some(namespace) = index.vector_namespace.as_deref() else {
            return Ok(Vec::new());
        };

        let embedder = build_embeddings_driver(
            &self.db,
            &self.provider_resolver,
            &self.driver_registry,
            index,
        )
        .await?;
        let response = embedder
            .driver
            .embed(
                &everruns_core::ProviderEndpoint::default(),
                EmbedRequest {
                    texts: vec![query.to_string()],
                    model: embedder.model_id,
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!("embedding request failed: {e}"))?;
        let embedding = response
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("embeddings provider returned no vector"))?;

        let matches = self
            .vector_store
            .query(
                namespace,
                VectorQuery {
                    vector: Some(embedding),
                    text: Some(query.to_string()),
                    top_k,
                },
            )
            .await?;
        Ok(matches.into_iter().map(|m| (m.id, m.score)).collect())
    }
}

#[async_trait]
impl KnowledgeIndexSearch for KnowledgeIndexSearchService {
    async fn search(
        &self,
        org_id: i64,
        index_ids: &[String],
        query: &str,
        top_k: usize,
    ) -> Result<Vec<KnowledgeIndexCitation>> {
        if top_k == 0 || query.trim().is_empty() || index_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Resolve the live, in-org indexes. Missing / cross-org / archived /
        // deleted ids are silently skipped to avoid leaking their existence.
        // `get_knowledge_index_by_public_id` is org-scoped and excludes deleted;
        // we additionally require `status == "active"` to exclude archived.
        let mut indexes: Vec<KnowledgeIndexRow> = Vec::new();
        for public_id in index_ids {
            let Some(index) = self
                .db
                .get_knowledge_index_by_public_id(org_id, public_id)
                .await?
            else {
                continue;
            };
            if index.org_id != org_id || index.status != "active" {
                continue;
            }
            indexes.push(index);
        }

        // Per-index hybrid query; collect each match tagged with its index.
        struct ScoredMatch {
            index_public_id: String,
            index_internal_id: uuid::Uuid,
            chunk_public_id: String,
            score: f32,
        }
        let mut all_matches: Vec<ScoredMatch> = Vec::new();
        for index in &indexes {
            let matches = self.query_index(index, query, top_k).await?;
            for (chunk_public_id, score) in matches {
                all_matches.push(ScoredMatch {
                    index_public_id: index.public_id.clone(),
                    index_internal_id: index.id,
                    chunk_public_id,
                    score,
                });
            }
        }

        // Global top_k by score (descending) across all bound indexes.
        all_matches.sort_by(|a, b| b.score.total_cmp(&a.score));
        all_matches.truncate(top_k);
        if all_matches.is_empty() {
            return Ok(Vec::new());
        }

        // Hydrate chunk text + document citation metadata per index.
        let mut citations: Vec<KnowledgeIndexCitation> = Vec::with_capacity(all_matches.len());
        for m in &all_matches {
            let hydrated = self
                .db
                .get_knowledge_index_chunks_with_documents(
                    m.index_internal_id,
                    std::slice::from_ref(&m.chunk_public_id),
                )
                .await?;
            let Some(chunk) = hydrated.into_iter().next() else {
                // Chunk vanished between vector match and hydration (re-sync
                // race); drop it rather than fabricate a citation.
                continue;
            };
            citations.push(KnowledgeIndexCitation {
                id: chunk.chunk_public_id,
                index_id: m.index_public_id.clone(),
                document_title: chunk.document_title,
                source_uri: chunk.source_uri,
                location: chunk.location,
                snippet: snippet_prefix(&chunk.text),
                score: m.score,
            });
        }

        Ok(citations)
    }
}

#[cfg(test)]
mod tests;

/// A trimmed prefix of the chunk passage for display.
fn snippet_prefix(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= SNIPPET_MAX_CHARS {
        return trimmed.to_string();
    }
    let prefix: String = trimmed.chars().take(SNIPPET_MAX_CHARS).collect();
    format!("{}…", prefix.trim_end())
}
