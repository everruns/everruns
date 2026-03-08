-- Everruns v0.8.4
-- Incremental migration for changes between v0.8.3 and v0.8.4
--
-- Includes:
-- - Index for unscoped app lookup by public_id (Slack webhook routing)
-- - Unique constraint on apps.public_id (global uniqueness, not just per-org)

-- ============================================
-- Apps: standalone public_id index
-- ============================================
-- The Slack webhook handler looks up apps by public_id without org scoping
-- (POST /v1/apps/{app_id}/slack/events). The existing composite index
-- idx_apps_org_public_id(org_id, public_id) cannot serve WHERE public_id = $1
-- efficiently because public_id is the second column. This standalone unique
-- index enables index-only lookups and enforces global uniqueness of public_ids.

CREATE UNIQUE INDEX IF NOT EXISTS idx_apps_public_id ON apps(public_id);
