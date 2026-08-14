// Generation reconciliation service.
//
// Queries llm_generations rows that have a provider_response_id but no
// reconciled_at, fetches authoritative usage/cost from the provider's API,
// and writes the result back. Currently only handles OpenRouter generations.
//
// Reconciliation is best-effort: failures are logged and do not affect
// the user turn or caller response.

use std::sync::Arc;

use super::openrouter_generation::fetch_openrouter_generation;
use anyhow::Result;
use everruns_provider::provider::DriverId;
use tracing::{debug, error, instrument, warn};

use crate::services::provider_resolver::resolve_provider_api_key;
use crate::storage::{EncryptionService, StorageBackend};

const OPENROUTER_PROVIDER: &str = "openrouter";
const RECONCILE_BATCH: i64 = 50;
const MISSING_KEY_RETRY_AFTER_SECONDS: i32 = 60 * 60;
const LOOKUP_FAILURE_RETRY_AFTER_SECONDS: i32 = 10 * 60;

pub struct GenerationReconcilerService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    http: reqwest::Client,
}

impl GenerationReconcilerService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self {
            db,
            encryption,
            http: reqwest::Client::new(),
        }
    }

    /// Reconcile a batch of unreconciled OpenRouter generations.
    ///
    /// Fetches up to `RECONCILE_BATCH` rows, groups them by org, resolves the
    /// org's OpenRouter API key, calls the OpenRouter generation API for each,
    /// and writes authoritative data back. Each row is processed independently;
    /// one failure does not abort the rest.
    #[instrument(skip(self), name = "generation_reconciler")]
    pub async fn reconcile_openrouter(&self) -> Result<()> {
        let rows = self
            .db
            .list_unreconciled_llm_generations(OPENROUTER_PROVIDER, RECONCILE_BATCH)
            .await?;

        if rows.is_empty() {
            return Ok(());
        }

        debug!(count = rows.len(), "reconciling OpenRouter generations");

        // Group by org to avoid redundant API-key lookups.
        let mut org_keys: std::collections::HashMap<i64, Option<String>> =
            std::collections::HashMap::new();

        for row in &rows {
            let api_key = match org_keys.entry(row.org_id) {
                std::collections::hash_map::Entry::Occupied(e) => e.get().clone(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let key = self.resolve_openrouter_key(row.org_id).await;
                    e.insert(key.clone());
                    key
                }
            };

            let Some(api_key) = api_key else {
                debug!(
                    org_id = row.org_id,
                    generation_id = %row.id,
                    retry_after_seconds = MISSING_KEY_RETRY_AFTER_SECONDS,
                    "no OpenRouter API key for org; delaying reconciliation"
                );
                self.delay_retry(row.id, MISSING_KEY_RETRY_AFTER_SECONDS)
                    .await;
                continue;
            };

            match fetch_openrouter_generation(&self.http, &api_key, &row.provider_response_id).await
            {
                Ok(authoritative) => {
                    if let Err(e) = self
                        .db
                        .reconcile_llm_generation(
                            row.id,
                            authoritative.tokens_prompt,
                            authoritative.tokens_completion,
                            authoritative.total_cost,
                            authoritative.provider_name.as_deref(),
                            authoritative.model.as_deref(),
                        )
                        .await
                    {
                        error!(
                            generation_id = %row.id,
                            provider_response_id = %row.provider_response_id,
                            error = %e,
                            "failed to write reconciliation result"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        generation_id = %row.id,
                        provider_response_id = %row.provider_response_id,
                        error = %e,
                        retry_after_seconds = LOOKUP_FAILURE_RETRY_AFTER_SECONDS,
                        "OpenRouter generation lookup failed; delaying retry"
                    );
                    self.delay_retry(row.id, LOOKUP_FAILURE_RETRY_AFTER_SECONDS)
                        .await;
                }
            }
        }

        Ok(())
    }

    async fn delay_retry(&self, id: uuid::Uuid, retry_after_seconds: i32) {
        if let Err(e) = self
            .db
            .mark_llm_generation_reconciliation_failed(id, retry_after_seconds)
            .await
        {
            error!(
                generation_id = %id,
                error = %e,
                "failed to record generation reconciliation retry delay"
            );
        }
    }

    async fn resolve_openrouter_key(&self, org_id: i64) -> Option<String> {
        let providers = match self.db.list_providers(org_id).await {
            Ok(p) => p,
            Err(e) => {
                error!(org_id, error = %e, "failed to list providers for reconciliation");
                return None;
            }
        };

        let openrouter = providers.into_iter().find(|p| {
            p.provider_type
                .parse::<DriverId>()
                .map(|t| t == DriverId::OpenRouter)
                .unwrap_or(false)
        })?;

        match resolve_provider_api_key(&self.db, self.encryption.as_deref(), &openrouter) {
            Ok(key) => key,
            Err(e) => {
                error!(org_id, error = %e, "failed to resolve OpenRouter API key for reconciliation");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use std::sync::Arc;

    fn make_reconciler() -> GenerationReconcilerService {
        let db = Arc::new(StorageBackend::in_memory());
        GenerationReconcilerService::new(db, None)
    }

    #[tokio::test]
    async fn reconcile_openrouter_no_rows_is_noop() {
        let svc = make_reconciler();
        // In-memory backend returns empty; should succeed without error.
        svc.reconcile_openrouter().await.unwrap();
    }
}
