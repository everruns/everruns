-- Org creation provisions settings and seed resources before required embedder
-- initializers run. They must not prevent rollback when an initializer fails.

ALTER TABLE harnesses
    DROP CONSTRAINT harnesses_org_id_fkey,
    ADD CONSTRAINT harnesses_org_id_fkey
        FOREIGN KEY (org_id) REFERENCES organizations(org_id) ON DELETE CASCADE;

ALTER TABLE plugin_marketplaces
    DROP CONSTRAINT plugin_marketplaces_org_id_fkey,
    ADD CONSTRAINT plugin_marketplaces_org_id_fkey
        FOREIGN KEY (org_id) REFERENCES organizations(org_id) ON DELETE CASCADE;

ALTER TABLE organization_settings
    DROP CONSTRAINT organization_settings_org_id_fkey,
    ADD CONSTRAINT organization_settings_org_id_fkey
        FOREIGN KEY (org_id) REFERENCES organizations(org_id) ON DELETE CASCADE;
