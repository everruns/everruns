-- One credential store serves end users, service virtual users, and organizations.
-- Organization connections are owned by hidden organization virtual users so
-- existing encryption, rotation, and leased-resource cleanup paths stay shared.
ALTER TABLE virtual_users DROP CONSTRAINT virtual_users_usage_check;
ALTER TABLE virtual_users ADD CONSTRAINT virtual_users_usage_check
    CHECK (usage IN ('end_user', 'service', 'organization'));

ALTER TABLE virtual_user_connections
    ADD COLUMN owner_scope TEXT NOT NULL DEFAULT 'virtual_user'
        CHECK (owner_scope IN ('virtual_user', 'organization')),
    ADD COLUMN name VARCHAR(255);

ALTER TABLE virtual_user_connections ADD CONSTRAINT organization_connection_name
    CHECK (owner_scope <> 'organization' OR name IS NOT NULL AND btrim(name) <> '');

ALTER TABLE leased_resources
    ADD COLUMN connection_id UUID REFERENCES virtual_user_connections(id) ON DELETE SET NULL;

COMMENT ON COLUMN virtual_user_connections.owner_scope IS
    'Product ownership scope. Each organization row is carried by a hidden organization virtual user.';
COMMENT ON COLUMN leased_resources.connection_id IS
    'Exact provider connection used to create the resource; cleanup must use the same account.';
