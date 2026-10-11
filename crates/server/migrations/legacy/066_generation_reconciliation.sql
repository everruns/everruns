-- Migration 059: OpenRouter generation ID and reconciliation tracking
--
-- Adds:
--   provider_response_id  - The generation ID returned by the provider (e.g.
--                           OpenRouter's "gen-..." identifier). Used to fetch
--                           authoritative usage/cost after the request completes.
--   reconciled_at         - Set when authoritative data has been written back.
--                           NULL means unreconciled. Used as idempotency guard.
--
-- The index on (provider, provider_response_id) supports the reconciliation
-- service query: "find unreconciled rows for provider X".

ALTER TABLE llm_generations
    ADD COLUMN provider_response_id TEXT,
    ADD COLUMN reconciled_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_llm_generations_provider_response_id
    ON llm_generations (provider, provider_response_id)
    WHERE provider_response_id IS NOT NULL AND reconciled_at IS NULL;
