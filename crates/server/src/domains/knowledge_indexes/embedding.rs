//! Shared embeddings-driver construction for Knowledge Indexes.
//!
//! Both the Syncout pipeline (`source_sync`) and the retrieval slice
//! (`search`) resolve an index's embedding model → provider → embeddings
//! service and build a driver from the registry. This module factors that path
//! so the two callers stay consistent (knowledge/runtime-resources/knowledge-indexes.md).

use std::sync::Arc;

use anyhow::{Result, anyhow};
use everruns_core::driver_registry::{
    BoxedEmbeddingsDriver, DriverRegistry, ProviderConfig, ServiceKind,
};

use crate::services::ProviderResolverService;
use crate::storage::StorageBackend;
use crate::storage::models::KnowledgeIndexRow;

/// An embeddings driver plus the provider-facing model id to pass to `embed`.
pub struct ResolvedEmbedder {
    pub driver: BoxedEmbeddingsDriver,
    /// Provider-side model identifier (e.g. `text-embedding-3-small`).
    pub model_id: String,
    /// Provider type that serves the call, named on usage records so embedding
    /// spend attributes to a provider like chat generations do (EVE-894).
    pub provider_type: String,
}

/// Resolve and build the embeddings driver for an index's embedding model.
///
/// Mirrors the resolution used at sync time: model → provider →
/// `resolve_service(org, Embeddings, Some(provider))` → registry driver.
pub async fn build_embeddings_driver(
    db: &StorageBackend,
    provider_resolver: &ProviderResolverService,
    driver_registry: &Arc<DriverRegistry>,
    index: &KnowledgeIndexRow,
) -> Result<ResolvedEmbedder> {
    let model = db
        .get_model(index.org_id, index.embedding_model_id.uuid())
        .await?
        .ok_or_else(|| anyhow!("embedding model not found"))?;
    let provider = db
        .get_provider(index.org_id, model.provider_id.uuid())
        .await?
        .ok_or_else(|| anyhow!("embedding provider not found"))?;

    let resolved = provider_resolver
        .resolve_service(
            index.org_id,
            ServiceKind::Embeddings,
            Some(&provider.id.to_string()),
        )
        .await?;
    let provider_type = resolved
        .provider_type
        .parse()
        .expect("DriverId::from_str is infallible");
    let provider_config = ProviderConfig {
        provider: everruns_core::ProviderKey::new(provider.id.to_string()),
        provider_type,
        api_key: Some(resolved.credentials.api_key),
        base_url: resolved.credentials.base_url,
        metadata: Default::default(),
    };
    let driver = driver_registry
        .create_embeddings_driver(&provider_config)
        .map_err(|e| anyhow!("failed to build embeddings driver: {e}"))?;

    Ok(ResolvedEmbedder {
        driver,
        model_id: model.model_id,
        provider_type: resolved.provider_type,
    })
}
