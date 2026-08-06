-- Org-level "default provider per service" resolution tier (EVE-569).
--
-- Tier 2 of service-bound provider resolution (knowledge/foundations/providers.md): consulted
-- after an explicit per-consumer binding and before the single-active-provider
-- fallback, an org may pin a default provider for a given ServiceKind (e.g.
-- provider X for `realtime`, provider Y for `embeddings`).
--
-- Stored as a JSONB object keyed by snake_case ServiceKind -> provider public id
-- (e.g. {"realtime": "provider_..."}). An empty object means "no org default";
-- resolution then falls through to the active-provider scan. A configured
-- default that is missing, inactive, or whose driver does not declare the
-- service is fail-closed at resolve time (it errors rather than falling
-- through). Purely additive; default deployments are unchanged.
ALTER TABLE organization_settings
ADD COLUMN default_provider_per_service JSONB NOT NULL DEFAULT '{}'::jsonb;
