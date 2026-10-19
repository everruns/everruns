-- Capture the authoritative per-generation cost reported by the provider.
--
-- OpenAI-compatible gateways such as OpenRouter return an inline `usage.cost`
-- (in USD credits, 1 credit = 1 USD) that reflects the real, post-routing
-- charge — accounting for provider routing, BYOK upstream pricing, and cache
-- discounts. This is more accurate than estimating from a static price table,
-- and is the only viable source for the long tail of models we do not maintain
-- price profiles for.
--
-- Nullable: providers that do not report a cost (direct OpenAI/Anthropic) leave
-- this NULL, and consumers fall back to the price-table estimate.
ALTER TABLE llm_generations
    ADD COLUMN cost_usd DOUBLE PRECISION;

COMMENT ON COLUMN llm_generations.cost_usd IS
    'Authoritative per-generation cost in USD as reported by the provider (e.g. OpenRouter usage.cost). NULL when the provider does not report a cost.';
