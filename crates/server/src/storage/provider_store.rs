// Database-backed ProviderStore implementation
//
// This module implements the core ProviderStore trait for retrieving
// LLM provider and model configurations from the database.
//
// Decision: org_id is baked into the struct at construction time,
// matching the Grpc/Adapter store pattern.

use async_trait::async_trait;
use everruns_core::{
    AgentLoopError, ModelId, Result, StoreResultExt,
    provider::DriverId,
    {provider_resolution::ProviderStore, provider_resolution::ResolvedModel},
};

use super::{encryption::EncryptionService, repositories::Database};

// ============================================================================
// DbProviderStore - Retrieves LLM provider configurations from database
// ============================================================================

/// Database-backed LLM provider store
///
/// Retrieves LLM model and provider configurations from the database,
/// including decrypted API keys.
///
/// Used by ReasonAtom to resolve model and provider info dynamically.
#[derive(Clone)]
pub struct DbProviderStore {
    db: Database,
    encryption: EncryptionService,
    org_id: i64,
}

impl DbProviderStore {
    pub fn new(db: Database, encryption: EncryptionService, org_id: i64) -> Self {
        Self {
            db,
            encryption,
            org_id,
        }
    }
}

#[async_trait]
impl ProviderStore for DbProviderStore {
    async fn get_resolved_model(&self, model_id: ModelId) -> Result<Option<ResolvedModel>> {
        // Look up the model
        let model_row = self
            .db
            .get_model(self.org_id, model_id.uuid())
            .await
            .store_err()?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Look up the provider
        let provider_row = self
            .db
            .get_provider(self.org_id, model_row.provider_id.uuid())
            .await
            .store_err()?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Parse provider type
        let provider_type = parse_provider_type(&provider_row.provider_type)?;

        Ok(Some(ResolvedModel {
            model: model_row.model_id,
            provider_type,
            api_key: None,
            base_url: None,
            provider_metadata: Some(everruns_core::ProviderMetadata {
                extra: Some(serde_json::json!({ "provider_id": provider_row.id.to_string() })),
                ..Default::default()
            }),
        }))
    }

    async fn get_default_model(&self) -> Result<Option<ResolvedModel>> {
        // Look up the default model via organization settings
        let model_row = self.db.get_default_model(self.org_id).await.store_err()?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Look up the provider
        let provider_row = self
            .db
            .get_provider(self.org_id, model_row.provider_id.uuid())
            .await
            .store_err()?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Parse provider type
        let provider_type = parse_provider_type(&provider_row.provider_type)?;

        Ok(Some(ResolvedModel {
            model: model_row.model_id,
            provider_type,
            api_key: None,
            base_url: None,
            provider_metadata: Some(everruns_core::ProviderMetadata {
                extra: Some(serde_json::json!({ "provider_id": provider_row.id.to_string() })),
                ..Default::default()
            }),
        }))
    }

    async fn get_provider_config(
        &self,
        provider: &everruns_core::ProviderKey,
    ) -> Result<Option<everruns_core::ProviderConfig>> {
        let id: everruns_core::ProviderId = provider.as_str().parse().map_err(|_| {
            AgentLoopError::Configuration(format!("invalid persisted provider id '{}'", provider))
        })?;
        let Some(row) = self
            .db
            .get_provider(self.org_id, id.uuid())
            .await
            .store_err()?
        else {
            return Ok(None);
        };
        let with_key = self
            .db
            .get_provider_with_api_key(&row, &self.encryption)
            .store_err()?;
        Ok(Some(everruns_core::ProviderConfig {
            provider: provider.clone(),
            provider_type: parse_provider_type(&with_key.provider_type)?,
            api_key: with_key.api_key,
            base_url: with_key.base_url,
            metadata: everruns_core::ProviderMetadata::default(),
        }))
    }
}

/// Parse a provider type string to its enum. Unknown non-empty ids map to the
/// open `External` variant so embedder-defined providers stored in the database
/// resolve to a usable provider type instead of erroring. An empty/whitespace
/// value is a corrupt row and surfaces as a configuration error rather than a
/// silent `External("")`.
fn parse_provider_type(provider_type_str: &str) -> Result<DriverId> {
    if provider_type_str.trim().is_empty() {
        return Err(AgentLoopError::Configuration(
            "empty provider_type in database".to_string(),
        ));
    }
    // FromStr is infallible: unknown ids become External.
    Ok(provider_type_str.parse().unwrap_or_else(|_| unreachable!()))
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed LLM provider store scoped to the given org
pub fn create_db_provider_store(
    db: Database,
    encryption: EncryptionService,
    org_id: i64,
) -> DbProviderStore {
    DbProviderStore::new(db, encryption, org_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_provider_type_returns_external_for_unknown_value() {
        // Unknown ids resolve to External, preserving the stored id.
        let parsed = parse_provider_type("totally-custom").unwrap();
        assert_eq!(parsed.to_string(), "totally-custom");
    }

    #[test]
    fn parse_provider_type_rejects_empty_value() {
        // Empty/whitespace is a corrupt row, not a valid external provider.
        assert!(parse_provider_type("").is_err());
        assert!(parse_provider_type("   ").is_err());
    }

    #[test]
    fn parse_provider_type_accepts_known_values() {
        assert_eq!(parse_provider_type("openai").unwrap().to_string(), "openai");
        assert_eq!(parse_provider_type("gemini").unwrap().to_string(), "gemini");
    }
}
