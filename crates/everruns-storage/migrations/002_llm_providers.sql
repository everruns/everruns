-- LLM Providers and Models
-- Decision: API keys encrypted with AES-256-GCM, key from SECRETS_ENCRYPTION_KEY env var
-- Decision: api_key_set flag indicates if key is configured (key never exposed in API)

-- LLM Providers table
CREATE TABLE llm_providers (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL CHECK (provider_type IN ('openai', 'anthropic', 'azure_openai', 'ollama', 'custom')),
    base_url TEXT,
    -- Encrypted API key (AES-256-GCM): 12-byte nonce || ciphertext || 16-byte tag
    api_key_encrypted BYTEA,
    api_key_set BOOLEAN NOT NULL DEFAULT FALSE,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_llm_providers_status ON llm_providers(status);
CREATE INDEX idx_llm_providers_provider_type ON llm_providers(provider_type);
-- Ensure only one default provider
CREATE UNIQUE INDEX idx_llm_providers_default ON llm_providers(is_default) WHERE is_default = TRUE;

-- LLM Models table
CREATE TABLE llm_models (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    provider_id UUID NOT NULL REFERENCES llm_providers(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    capabilities JSONB NOT NULL DEFAULT '[]'::jsonb,
    context_window INTEGER,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_llm_models_provider_id ON llm_models(provider_id);
CREATE INDEX idx_llm_models_status ON llm_models(status);
-- Unique model_id per provider
CREATE UNIQUE INDEX idx_llm_models_provider_model ON llm_models(provider_id, model_id);
-- Ensure only one default model per provider
CREATE UNIQUE INDEX idx_llm_models_provider_default ON llm_models(provider_id, is_default) WHERE is_default = TRUE;

-- Apply updated_at trigger
CREATE TRIGGER update_llm_providers_updated_at BEFORE UPDATE ON llm_providers
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_llm_models_updated_at BEFORE UPDATE ON llm_models
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
