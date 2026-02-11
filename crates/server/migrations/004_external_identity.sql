-- Support for external identity providers (PropelAuth, Auth0, etc.)
-- The external_id column maps external provider user/org IDs to internal IDs.
-- OSS users: unused (NULL). SaaS: populated by auth backend sync.

ALTER TABLE users ADD COLUMN external_id TEXT UNIQUE;
ALTER TABLE organizations ADD COLUMN external_id TEXT UNIQUE;

CREATE INDEX idx_users_external_id ON users(external_id) WHERE external_id IS NOT NULL;
CREATE INDEX idx_organizations_external_id ON organizations(external_id) WHERE external_id IS NOT NULL;
