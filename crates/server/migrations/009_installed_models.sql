-- Migration: Split available vs installed models + organization settings
--
-- Changes:
-- 1. Add `installed` boolean to llm_models (default false for existing models)
-- 2. Create organization_settings table with default_model_id
-- 3. Migrate is_default data to organization_settings
-- 4. Drop is_default column and its unique index from llm_models

-- Step 1: Add installed column (all existing models start as not installed)
ALTER TABLE llm_models ADD COLUMN installed BOOLEAN NOT NULL DEFAULT FALSE;

-- Step 2: Mark historically "important" models as installed
-- This will be handled by seed data on next startup. For existing deployments,
-- mark all favorite models as installed for continuity.
UPDATE llm_models SET installed = TRUE WHERE is_favorite = TRUE;

-- Step 3: Create organization_settings table
CREATE TABLE organization_settings (
    org_id BIGINT PRIMARY KEY REFERENCES organizations(org_id),
    default_model_id UUID REFERENCES llm_models(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Step 4: Migrate is_default data to organization_settings
INSERT INTO organization_settings (org_id, default_model_id)
SELECT DISTINCT m.org_id, m.id
FROM llm_models m
WHERE m.is_default = TRUE
ON CONFLICT (org_id) DO UPDATE SET default_model_id = EXCLUDED.default_model_id;

-- Step 5: Ensure every org has a settings row (even without a default model)
INSERT INTO organization_settings (org_id)
SELECT org_id FROM organizations
ON CONFLICT (org_id) DO NOTHING;

-- Step 6: Drop is_default column and its unique index
DROP INDEX IF EXISTS idx_llm_models_default;
ALTER TABLE llm_models DROP COLUMN is_default;

-- Step 7: Index on installed for efficient filtering
CREATE INDEX idx_llm_models_installed ON llm_models(installed) WHERE installed = TRUE;
