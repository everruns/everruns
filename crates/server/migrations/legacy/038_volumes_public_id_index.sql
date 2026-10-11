-- Add a standalone unique index on volumes(public_id) to support unscoped
-- lookups by public_id (e.g. /v1/resolve-org for vol_* IDs). Volumes already
-- has a composite UNIQUE index on (org_id, public_id), but the planner cannot
-- use it efficiently for equality on the trailing column alone. This standalone
-- unique index enables index-only lookups and enforces global uniqueness of
-- volume public_ids, mirroring the pattern in 005_v0.8.4.sql for apps.

CREATE UNIQUE INDEX IF NOT EXISTS idx_volumes_public_id ON volumes(public_id);
