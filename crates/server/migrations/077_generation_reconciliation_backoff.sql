-- Migration 077: OpenRouter generation reconciliation retry backoff
--
-- Failed generation reconciliation attempts are delayed before they are
-- eligible for another batch. This prevents permanently failing oldest rows
-- from monopolizing every global reconciliation batch.

ALTER TABLE llm_generations
    ADD COLUMN reconciliation_attempts INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN reconcile_after TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_llm_generations_reconcile_ready
    ON llm_generations (provider, reconcile_after, created_at)
    WHERE provider_response_id IS NOT NULL AND reconciled_at IS NULL;
