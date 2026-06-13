-- Providers domain model rename (specs/providers.md):
-- llm_providers -> providers, llm_models -> models.
--
-- Tables and their indexes are renamed in place; data, constraints, and FKs
-- are preserved. Column renames (provider_type -> driver, api_key_* ->
-- credentials_*) land together with the API surface rename in a later phase
-- so wire names, column names, and Rust field names move as one.

ALTER TABLE llm_providers RENAME TO providers;
ALTER TABLE llm_models RENAME TO models;

ALTER INDEX IF EXISTS idx_llm_providers_status RENAME TO idx_providers_status;
ALTER INDEX IF EXISTS idx_llm_providers_provider_type RENAME TO idx_providers_provider_type;
ALTER INDEX IF EXISTS idx_llm_providers_org_id RENAME TO idx_providers_org_id;

ALTER INDEX IF EXISTS idx_llm_models_provider_id RENAME TO idx_models_provider_id;
ALTER INDEX IF EXISTS idx_llm_models_org_id RENAME TO idx_models_org_id;
ALTER INDEX IF EXISTS idx_llm_models_provider_model RENAME TO idx_models_provider_model;
ALTER INDEX IF EXISTS idx_llm_models_favorite RENAME TO idx_models_favorite;
ALTER INDEX IF EXISTS idx_llm_models_source RENAME TO idx_models_source;
ALTER INDEX IF EXISTS idx_llm_models_enabled RENAME TO idx_models_enabled;

ALTER TABLE providers RENAME CONSTRAINT llm_providers_pkey TO providers_pkey;
ALTER TABLE models RENAME CONSTRAINT llm_models_pkey TO models_pkey;
