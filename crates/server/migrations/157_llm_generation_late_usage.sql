-- Late usage of OpenAI Agents API turns (EVE-1145).
--
-- The Agents API fills a turn's usage after `turn.completed`. The durable
-- driver re-reads the turn a few times, and a turn whose usage is still null
-- is billed as an explicit unknown (a `model_tokens` cost component with no
-- amount). Such a generation is recorded with zero tokens and
-- `usage_pending`, plus what it takes to read the turn back later: the
-- provider session (`provider_session_id`) and the Everruns provider whose
-- credentials ran it (`provider_config_id`, its public id). The turn id is
-- the existing `provider_response_id`.
--
-- The reconciler clears `usage_pending` in the same statement that writes the
-- usage, so the flag is the idempotency guard against a second debit. Failed
-- lookups reuse the existing `reconciliation_attempts` / `reconcile_after`
-- backoff columns.

ALTER TABLE llm_generations
    ADD COLUMN provider_session_id TEXT,
    ADD COLUMN provider_config_id TEXT,
    ADD COLUMN usage_pending BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN llm_generations.usage_pending IS
    'Usage the provider had not reported when the generation was billed; cleared once the late usage is applied (EVE-1145).';

CREATE INDEX IF NOT EXISTS idx_llm_generations_usage_pending
    ON llm_generations (reconcile_after, created_at)
    WHERE usage_pending;
