//! Hosted knowledge retrieval service contracts.
//!
//! The execution kernel carries only a generic typed extension bag. Product
//! projection installs these narrow, tenant-scoped services when the selected
//! capability family is available.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::error::Result;
use everruns_core::typed_id::OrgId;

use crate::vector_store::{KnowledgeIndexCitation, KnowledgeIndexSearch};

/// One ranked hit from an organization Knowledge Base search.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KnowledgeSearchHit {
    /// Entry public ID (`kbe_…`).
    pub id: String,
    /// Owning Knowledge Base public ID (`kb_…`).
    pub kb_id: String,
    /// Human-readable result title.
    pub title: String,
    /// Knowledge entry kind.
    pub kind: String,
    /// Normalized entry tags.
    pub tags: Vec<String>,
    /// Bounded body excerpt for the model.
    pub snippet: String,
    /// Optional OKF resource URI.
    pub resource: Option<String>,
}

/// Tenant-scoped search over curated Knowledge Bases.
///
/// Implementations must scope every lookup to `org_id` and silently ignore
/// requested Knowledge Base IDs owned by another organization so callers
/// cannot use result differences as a cross-tenant existence oracle.
/// THREAT[TM-TENANT-001]: the service boundary carries the required tenant key.
#[async_trait]
pub trait KnowledgeStore: Send + Sync {
    async fn search_knowledge(
        &self,
        org_id: OrgId,
        kb_public_ids: &[String],
        query: &str,
        kind: Option<&str>,
        tags: &[String],
        limit: usize,
    ) -> Result<Vec<KnowledgeSearchHit>>;
}

/// Typed tool-context extension for Knowledge Base retrieval.
#[derive(Clone)]
pub struct KnowledgeStoreExt(pub Arc<dyn KnowledgeStore>);

/// Typed tool-context extension for Knowledge Index retrieval.
#[derive(Clone)]
pub struct KnowledgeIndexSearchExt(pub Arc<dyn KnowledgeIndexSearch>);

/// Citation result returned by the hosted Knowledge Index search service.
pub type KnowledgeIndexSearchHit = KnowledgeIndexCitation;
