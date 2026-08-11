//! Vendor-neutral vector store abstraction for Knowledge Indexes.
//!
//! See `knowledge/runtime-resources/knowledge-indexes.md`. Embedding vectors for Knowledge Index
//! chunks live in an external vector database, not in Postgres. This module
//! defines the platform-selected `VectorStore` trait (mirroring how
//! `SessionFileSystemFactory` keeps the filesystem pluggable) plus an
//! `InMemoryVectorStore` used by dev mode and storage-parity tests.
//!
//! The store is **multitenant and multi-index** by construction: one namespace
//! per index, org-prefixed for isolation. Callers derive the namespace with
//! [`index_namespace`] and must validate the index's `org_id` before issuing a
//! call so cross-org reads are structurally impossible. The reference
//! production backend (Turbopuffer) maps each namespace onto a Turbopuffer
//! namespace.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Type-keyed wrapper installed on a [`everruns_core::PlatformDefinition`] by
/// hosted presets. Core carries the generic extension bag but does not name the
/// Knowledge Index service.
#[derive(Clone)]
pub struct VectorStoreExt(pub Arc<dyn VectorStore>);

/// Reciprocal-rank-fusion constant for hybrid (vector + text) queries.
const RRF_K: f32 = 60.0;

/// A single retrieved chunk returned by [`KnowledgeIndexSearch::search`], shaped
/// to the `search_index` citation contract in `knowledge/runtime-resources/knowledge-indexes.md`. The
/// `id` (`kchk_…`) + `source_uri` + `location` give the agent a stable, linkable
/// citation. Kept compatible with the planned `search_knowledge` citation shape
/// so the UI can render both uniformly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeIndexCitation {
    /// Chunk `public_id` (`kchk_…`). The stable citation id.
    pub id: String,
    /// Owning Knowledge Index `public_id` (`kidx_…`).
    pub index_id: String,
    /// Title of the source document, if known.
    pub document_title: Option<String>,
    /// Stable per-source locator (e.g. `github://owner/repo@main/docs/x.md`).
    pub source_uri: String,
    /// Provenance within the document (line / char / page ranges).
    pub location: Option<serde_json::Value>,
    /// A trimmed prefix of the chunk passage.
    pub snippet: String,
    /// Relevance score; higher is more relevant. Use for ordering only.
    pub score: f32,
}

/// Server-implemented hybrid retrieval over an org's bound Knowledge Indexes.
///
/// Carried on `ToolContext` so the `search_index` agent tool can reach the
/// server-side embedding + vector-store machinery without `everruns-core`
/// depending on server types (mirrors `PlatformStore` / `UserConnectionResolver`).
#[async_trait]
pub trait KnowledgeIndexSearch: Send + Sync {
    /// Embed `query`, run hybrid retrieval against each bound index's
    /// vector-store namespace, and return up to `top_k` citations ordered by
    /// score (descending).
    ///
    /// `index_ids` are the `kidx_` public ids bound in the capability config.
    /// Ids that are missing, cross-org, archived, or deleted are silently
    /// skipped — no existence leak.
    async fn search(
        &self,
        org_id: i64,
        index_ids: &[String],
        query: &str,
        top_k: usize,
    ) -> Result<Vec<KnowledgeIndexCitation>>;
}

/// Derive the vector-store namespace for an index.
///
/// Org-prefixed so every query targets a single, org-derived namespace and
/// cross-org reads cannot happen. `public_id` is the `kidx_…` index id.
pub fn index_namespace(org_id: i64, public_id: &str) -> String {
    format!("org_{org_id}__{public_id}")
}

/// A single point stored in the vector store, keyed by the chunk `public_id`
/// (`kchk_…`), which is also the stable citation id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorRecord {
    /// Chunk `public_id` (`kchk_…`). Stable citation id and primary key.
    pub id: String,
    /// Embedding for the chunk. Length must match the index's `vector_dim`.
    pub vector: Vec<f32>,
    /// Chunk passage text, stored to enable BM25 full-text scoring.
    pub text: String,
    /// Owning document `public_id` (`kidoc_…`), used for bulk delete on re-sync.
    pub document_id: String,
}

/// A retrieval request against a single namespace. At least one of `vector`
/// (semantic KNN) or `text` (BM25) must be set; when both are present the
/// backend fuses them with reciprocal-rank fusion.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VectorQuery {
    /// Query embedding for semantic KNN.
    pub vector: Option<Vec<f32>>,
    /// Query text for full-text (BM25) scoring.
    pub text: Option<String>,
    /// Maximum number of matches to return.
    pub top_k: usize,
}

/// A ranked match returned from [`VectorStore::query`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorMatch {
    /// Chunk `public_id` (`kchk_…`) — hydrate text/citation from Postgres.
    pub id: String,
    /// Owning document `public_id` (`kidoc_…`).
    pub document_id: String,
    /// Higher is more relevant. Scale is backend-defined; use for ordering only.
    pub score: f32,
}

/// Pluggable embedding store for Knowledge Indexes. Selected through
/// `PlatformDefinition`; the in-memory backend backs dev/tests, Turbopuffer is
/// the reference production backend.
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Insert or replace records in a namespace, keyed by `VectorRecord::id`.
    async fn upsert(&self, namespace: &str, records: Vec<VectorRecord>) -> Result<()>;

    /// Rank records in a namespace against the query. Returns up to
    /// `query.top_k` matches, most relevant first.
    async fn query(&self, namespace: &str, query: VectorQuery) -> Result<Vec<VectorMatch>>;

    /// Remove every record belonging to a document (used on document re-sync).
    async fn delete_by_document(&self, namespace: &str, document_id: &str) -> Result<()>;

    /// Drop an entire namespace (used on index hard delete).
    async fn delete_namespace(&self, namespace: &str) -> Result<()>;
}

/// In-memory brute-force `VectorStore` for dev mode and storage-parity tests.
///
/// Vector ranking uses cosine similarity; text ranking uses a simple
/// term-overlap score standing in for BM25. Hybrid queries fuse the two ranked
/// lists with reciprocal-rank fusion. Not for production scale.
#[derive(Default)]
pub struct InMemoryVectorStore {
    namespaces: Mutex<HashMap<String, Vec<VectorRecord>>>,
}

impl InMemoryVectorStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl VectorStore for InMemoryVectorStore {
    async fn upsert(&self, namespace: &str, records: Vec<VectorRecord>) -> Result<()> {
        let mut store = self.namespaces.lock().expect("vector store poisoned");
        let entry = store.entry(namespace.to_string()).or_default();
        for record in records {
            if let Some(existing) = entry.iter_mut().find(|r| r.id == record.id) {
                *existing = record;
            } else {
                entry.push(record);
            }
        }
        Ok(())
    }

    async fn query(&self, namespace: &str, query: VectorQuery) -> Result<Vec<VectorMatch>> {
        if query.top_k == 0 {
            return Ok(Vec::new());
        }
        let store = self.namespaces.lock().expect("vector store poisoned");
        let Some(records) = store.get(namespace) else {
            return Ok(Vec::new());
        };

        let vector_ranked = query
            .vector
            .as_ref()
            .map(|v| rank_by(records, |r| cosine_similarity(&r.vector, v)));
        let text_ranked = query.text.as_ref().and_then(|t| {
            let t = t.trim();
            (!t.is_empty()).then(|| rank_by(records, |r| term_overlap_score(&r.text, t)))
        });

        let ordered_ids = match (vector_ranked, text_ranked) {
            (Some(v), Some(t)) => fuse_rrf(&v, &t),
            (Some(v), None) => v,
            (None, Some(t)) => t,
            // No query signal: nothing to rank against.
            (None, None) => return Ok(Vec::new()),
        };

        let by_id: HashMap<&str, &VectorRecord> =
            records.iter().map(|r| (r.id.as_str(), r)).collect();
        let matches = ordered_ids
            .into_iter()
            .take(query.top_k)
            .filter_map(|(id, score)| {
                by_id.get(id.as_str()).map(|r| VectorMatch {
                    id: r.id.clone(),
                    document_id: r.document_id.clone(),
                    score,
                })
            })
            .collect();
        Ok(matches)
    }

    async fn delete_by_document(&self, namespace: &str, document_id: &str) -> Result<()> {
        let mut store = self.namespaces.lock().expect("vector store poisoned");
        if let Some(entry) = store.get_mut(namespace) {
            entry.retain(|r| r.document_id != document_id);
        }
        Ok(())
    }

    async fn delete_namespace(&self, namespace: &str) -> Result<()> {
        let mut store = self.namespaces.lock().expect("vector store poisoned");
        store.remove(namespace);
        Ok(())
    }
}

/// Rank records by a scoring function, descending. Returns `(id, score)` pairs.
fn rank_by(records: &[VectorRecord], score: impl Fn(&VectorRecord) -> f32) -> Vec<(String, f32)> {
    let mut scored: Vec<(String, f32)> = records
        .iter()
        .map(|r| (r.id.clone(), score(r)))
        .filter(|(_, s)| *s > f32::NEG_INFINITY)
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored
}

/// Fuse two ranked lists with reciprocal-rank fusion, returning ids ordered by
/// fused score (descending). The score carried out is the RRF score.
fn fuse_rrf(a: &[(String, f32)], b: &[(String, f32)]) -> Vec<(String, f32)> {
    let mut fused: HashMap<&str, f32> = HashMap::new();
    for list in [a, b] {
        for (rank, (id, _)) in list.iter().enumerate() {
            *fused.entry(id.as_str()).or_insert(0.0) += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
    }
    let mut ranked: Vec<(String, f32)> = fused
        .into_iter()
        .map(|(id, s)| (id.to_string(), s))
        .collect();
    ranked.sort_by(|x, y| y.1.total_cmp(&x.1));
    ranked
}

/// Cosine similarity in [-1, 1]; mismatched dimensions or zero vectors score
/// as the minimum so they sort last.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return f32::NEG_INFINITY;
    }
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return f32::NEG_INFINITY;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// Fraction of distinct query terms present in the text (case-insensitive).
/// A lightweight stand-in for BM25 in the in-memory backend.
fn term_overlap_score(text: &str, query: &str) -> f32 {
    let haystack = text.to_lowercase();
    let terms: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if terms.is_empty() {
        return f32::NEG_INFINITY;
    }
    let hits = terms.iter().filter(|t| haystack.contains(*t)).count();
    if hits == 0 {
        f32::NEG_INFINITY
    } else {
        hits as f32 / terms.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, doc: &str, vector: Vec<f32>, text: &str) -> VectorRecord {
        VectorRecord {
            id: id.to_string(),
            vector,
            text: text.to_string(),
            document_id: doc.to_string(),
        }
    }

    #[test]
    fn namespace_is_org_prefixed() {
        assert_eq!(
            index_namespace(1, "kidx_00000000000000000000000000000001"),
            "org_1__kidx_00000000000000000000000000000001"
        );
    }

    #[tokio::test]
    async fn vector_query_ranks_by_cosine() {
        let store = InMemoryVectorStore::new();
        let ns = index_namespace(1, "kidx_00000000000000000000000000000001");
        store
            .upsert(
                &ns,
                vec![
                    record("kchk_a", "kidoc_1", vec![1.0, 0.0], "alpha"),
                    record("kchk_b", "kidoc_1", vec![0.0, 1.0], "beta"),
                    record("kchk_c", "kidoc_2", vec![0.9, 0.1], "gamma"),
                ],
            )
            .await
            .unwrap();

        let matches = store
            .query(
                &ns,
                VectorQuery {
                    vector: Some(vec![1.0, 0.0]),
                    text: None,
                    top_k: 2,
                },
            )
            .await
            .unwrap();

        let ids: Vec<_> = matches.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["kchk_a", "kchk_c"]);
        assert_eq!(matches[0].document_id, "kidoc_1");
    }

    #[tokio::test]
    async fn upsert_replaces_existing_id() {
        let store = InMemoryVectorStore::new();
        let ns = "org_1__kidx_x";
        store
            .upsert(ns, vec![record("kchk_a", "kidoc_1", vec![1.0, 0.0], "old")])
            .await
            .unwrap();
        store
            .upsert(ns, vec![record("kchk_a", "kidoc_1", vec![0.0, 1.0], "new")])
            .await
            .unwrap();

        let matches = store
            .query(
                ns,
                VectorQuery {
                    vector: Some(vec![0.0, 1.0]),
                    text: None,
                    top_k: 5,
                },
            )
            .await
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert!(matches[0].score > 0.99);
    }

    #[tokio::test]
    async fn text_query_ranks_by_term_overlap() {
        let store = InMemoryVectorStore::new();
        let ns = "org_1__kidx_x";
        store
            .upsert(
                ns,
                vec![
                    record("kchk_a", "kidoc_1", vec![0.0], "the quick brown fox"),
                    record("kchk_b", "kidoc_1", vec![0.0], "a slow green turtle"),
                ],
            )
            .await
            .unwrap();

        let matches = store
            .query(
                ns,
                VectorQuery {
                    vector: None,
                    text: Some("quick fox".to_string()),
                    top_k: 5,
                },
            )
            .await
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "kchk_a");
    }

    #[tokio::test]
    async fn delete_by_document_and_namespace() {
        let store = InMemoryVectorStore::new();
        let ns = "org_1__kidx_x";
        store
            .upsert(
                ns,
                vec![
                    record("kchk_a", "kidoc_1", vec![1.0], "a"),
                    record("kchk_b", "kidoc_2", vec![1.0], "b"),
                ],
            )
            .await
            .unwrap();

        store.delete_by_document(ns, "kidoc_1").await.unwrap();
        let after = store
            .query(
                ns,
                VectorQuery {
                    vector: Some(vec![1.0]),
                    text: None,
                    top_k: 5,
                },
            )
            .await
            .unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, "kchk_b");

        store.delete_namespace(ns).await.unwrap();
        let empty = store
            .query(
                ns,
                VectorQuery {
                    vector: Some(vec![1.0]),
                    text: None,
                    top_k: 5,
                },
            )
            .await
            .unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn hybrid_query_fuses_both_signals() {
        let store = InMemoryVectorStore::new();
        let ns = "org_1__kidx_x";
        store
            .upsert(
                ns,
                vec![
                    record(
                        "kchk_a",
                        "kidoc_1",
                        vec![1.0, 0.0],
                        "database indexing guide",
                    ),
                    record("kchk_b", "kidoc_1", vec![0.0, 1.0], "cooking recipes"),
                ],
            )
            .await
            .unwrap();

        let matches = store
            .query(
                ns,
                VectorQuery {
                    vector: Some(vec![1.0, 0.0]),
                    text: Some("database".to_string()),
                    top_k: 2,
                },
            )
            .await
            .unwrap();
        assert_eq!(matches[0].id, "kchk_a");
    }
}
