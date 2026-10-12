-- Managed vs own-key spend on every generation.
--
-- `provider_config_id` (added for late Agents API usage in 158) now records
-- the provider account that served every call, not only pending ones. The
-- new `managed` flag copies that provider's host-managed bit (EVE-810) at the
-- time of the call, so usage, budgets and reports can tell spend on
-- host-managed models (billed by the host) apart from spend on the org's own
-- keys (BYOK) without joining a provider row that may since have changed or
-- been deleted. Rows written before this migration read as not managed.

ALTER TABLE llm_generations
    ADD COLUMN IF NOT EXISTS managed BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN llm_generations.provider_config_id IS
    'Public id of the provider account that served the call; NULL when the host resolved no stored provider.';
COMMENT ON COLUMN llm_generations.managed IS
    'Whether the serving provider was host-managed at the time of the call (managed spend vs BYOK).';
