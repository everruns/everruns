-- Preserve whether an agent selected a harness or inherits the organization default.
-- Existing agents owned their persisted harness before this distinction existed,
-- so they remain explicit for backward-compatible runtime behavior.

ALTER TABLE agents
    ADD COLUMN harness_source VARCHAR(32) NOT NULL DEFAULT 'explicit'
    CHECK (harness_source IN ('explicit', 'organization_default'));
