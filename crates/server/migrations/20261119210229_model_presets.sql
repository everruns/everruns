-- Model presets: named sets of models, one per job (default, chat, coding,
-- research, small, decision). The organization defaults stay the baseline;
-- an agent may follow a preset instead. Built-in presets (Automatic and one
-- per connected provider) are computed from curated provider picks and never
-- stored; only custom presets live here. A NULL job means "organization
-- default" for that job. Deleting a model clears the job rather than the
-- preset. Presets are soft deleted (records are kept).
-- See knowledge/foundations/model-presets.md.
CREATE TABLE model_presets (
    id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    key TEXT NOT NULL CHECK (key ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    description TEXT,
    default_model_id UUID REFERENCES models(id) ON DELETE SET NULL,
    chat_model_id UUID REFERENCES models(id) ON DELETE SET NULL,
    coding_model_id UUID REFERENCES models(id) ON DELETE SET NULL,
    research_model_id UUID REFERENCES models(id) ON DELETE SET NULL,
    small_model_id UUID REFERENCES models(id) ON DELETE SET NULL,
    decision_model_id UUID REFERENCES models(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX model_presets_org_key_live
    ON model_presets (org_id, key) WHERE deleted_at IS NULL;

-- Organization default for coding work (coding-harness agents and the
-- `coding` helper). NULL falls back to the default model.
ALTER TABLE organization_settings
    ADD COLUMN coding_model_id UUID REFERENCES models(id) ON DELETE SET NULL;

-- The preset an agent follows: a built-in key (`automatic`,
-- `provider:<driver>`) or a custom preset's key. NULL = organization defaults.
ALTER TABLE agents ADD COLUMN model_preset TEXT;
