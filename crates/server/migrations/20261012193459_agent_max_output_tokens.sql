-- agent_max_output_tokens
-- Per-agent ceiling on output tokens per model response, sent to the provider
-- as max_tokens. NULL keeps the provider default.

ALTER TABLE agents
    ADD COLUMN max_output_tokens INTEGER,
    ADD CONSTRAINT agents_max_output_tokens_positive CHECK (max_output_tokens IS NULL OR max_output_tokens > 0);

COMMENT ON COLUMN agents.max_output_tokens IS
    'Per-response output token ceiling (max_tokens). NULL: provider default.';
