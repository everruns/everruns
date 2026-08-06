-- Model Routers (EVE-397)
-- See knowledge/integrations/model-router.md for full design intent.
--
-- A Model Router owns named routes (e.g. base, utility, analysis, review),
-- each with a selection strategy and ordered candidates that point at
-- concrete LLM models with optional request overrides.
--
-- This migration adds the foundational schema:
--   - model_routers:           the router entity (lifecycle: active/archived/deleted)
--   - model_router_routes:     named routes inside a router (key, purpose, when_to_use, strategy)
--   - model_router_candidates: ordered candidates inside a route (concrete model + overrides + weight/rules)
--
-- Storage trait, domain CRUD, REST API, runtime resolver, harness/agent/
-- session binding migrations, and UI ship as follow-up vertical slices on
-- top of this foundation. Today's concrete `default_model_id` columns are
-- left unchanged so existing model selection behavior is preserved.

-- ============================================
-- Model Routers
-- ============================================

CREATE TABLE model_routers (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    -- Dual-ID pattern: internal UUID (id) + external public_id (API-facing)
    public_id TEXT NOT NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    -- JSON Schema describing caller-supplied params validated at binding time.
    -- Default `{}` means "no parameters expected".
    param_schema JSONB NOT NULL DEFAULT '{}'::jsonb,
    status VARCHAR(50) NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'archived', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,

    -- Format validation: mrtr_ prefix + 32 lowercase hex chars
    CONSTRAINT model_routers_public_id_format
        CHECK (public_id ~ '^mrtr_[0-9a-f]{32}$')
);

CREATE INDEX idx_model_routers_org_id ON model_routers(org_id);
CREATE INDEX idx_model_routers_status ON model_routers(status);
-- Unique per org (same public_id allowed across orgs)
CREATE UNIQUE INDEX idx_model_routers_org_public_id
    ON model_routers(org_id, public_id);
-- Unique name per org while not deleted
CREATE UNIQUE INDEX idx_model_routers_org_name_active
    ON model_routers(org_id, name)
    WHERE status != 'deleted';

CREATE TRIGGER update_model_routers_updated_at BEFORE UPDATE ON model_routers
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE model_routers IS 'Model Routers — semantic LLM selection. See knowledge/integrations/model-router.md.';

-- ============================================
-- Model Router Routes
-- ============================================
-- Named routes inside a router (e.g. `base`, `utility`, `analysis`, `review`).
-- Routes carry the human-facing `purpose` and the model-facing `when_to_use`
-- description used by the future `set_model` discoverability tool. They are
-- flat fields, not wrapped in a `semantic` object.

CREATE TABLE model_router_routes (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    router_id UUID NOT NULL REFERENCES model_routers(id) ON DELETE CASCADE,

    -- Stable identifier within router (e.g. `base`, `analysis`).
    -- Lowercase letters, digits, and hyphens only.
    key VARCHAR(64) NOT NULL,
    purpose TEXT NOT NULL,
    when_to_use TEXT NOT NULL,
    strategy VARCHAR(32) NOT NULL
        CHECK (strategy IN ('single', 'ordered_fallback', 'weighted', 'rules', 'custom')),
    position INTEGER NOT NULL DEFAULT 0,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT model_router_routes_key_format
        CHECK (key ~ '^[a-z0-9]([a-z0-9-]*[a-z0-9])?$')
);

-- Unique key per router
CREATE UNIQUE INDEX idx_model_router_routes_router_key
    ON model_router_routes(router_id, key);
CREATE INDEX idx_model_router_routes_router_id
    ON model_router_routes(router_id);
CREATE INDEX idx_model_router_routes_router_position
    ON model_router_routes(router_id, position);

CREATE TRIGGER update_model_router_routes_updated_at
    BEFORE UPDATE ON model_router_routes
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE model_router_routes IS 'Named routes inside a Model Router (key, purpose, when_to_use, strategy).';

-- ============================================
-- Model Router Candidates
-- ============================================
-- Ordered candidates inside a route. Each candidate references a concrete
-- model and may carry provider-agnostic request overrides
-- (reasoning_effort, temperature, max_output_tokens, ...). `weight` is used
-- by the `weighted` strategy; `rules` is used by the `rules` strategy.

CREATE TABLE model_router_candidates (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    route_id UUID NOT NULL REFERENCES model_router_routes(id) ON DELETE CASCADE,
    model_id UUID NOT NULL REFERENCES llm_models(id),

    -- Provider-agnostic overrides applied at LLM-call time.
    request_overrides JSONB NOT NULL DEFAULT '{}'::jsonb,

    -- Used by `weighted` strategy. Default 1 = uniform weight.
    weight INTEGER NOT NULL DEFAULT 1
        CHECK (weight >= 0),

    -- Used by `rules` strategy. Free-form rules document validated by the
    -- server when the strategy is `rules`.
    rules JSONB,

    -- Used by `ordered_fallback` strategy.
    position INTEGER NOT NULL DEFAULT 0,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_model_router_candidates_route_id
    ON model_router_candidates(route_id);
CREATE INDEX idx_model_router_candidates_model_id
    ON model_router_candidates(model_id);
CREATE INDEX idx_model_router_candidates_route_position
    ON model_router_candidates(route_id, position);

CREATE TRIGGER update_model_router_candidates_updated_at
    BEFORE UPDATE ON model_router_candidates
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE model_router_candidates IS 'Ordered candidates inside a Model Router route (concrete model + overrides + weight/rules).';
