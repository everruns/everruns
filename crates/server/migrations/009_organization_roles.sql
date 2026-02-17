-- Organization member roles
-- Adds hierarchical role column: owner > admin > member

ALTER TABLE organization_members
    ADD COLUMN role TEXT NOT NULL DEFAULT 'member'
    CHECK (role IN ('owner', 'admin', 'member'));

CREATE INDEX idx_organization_members_role ON organization_members(org_id, role);

-- Track who created each organization
ALTER TABLE organizations
    ADD COLUMN created_by UUID REFERENCES users(id);
