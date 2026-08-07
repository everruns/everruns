-- Org creation provisions these rows before required embedder initializers run.
-- They must not prevent the creation rollback when an initializer fails.

ALTER TABLE harnesses
    DROP CONSTRAINT harnesses_org_id_fkey,
    ADD CONSTRAINT harnesses_org_id_fkey
        FOREIGN KEY (org_id) REFERENCES organizations(org_id) ON DELETE CASCADE;

ALTER TABLE plugin_marketplaces
    DROP CONSTRAINT plugin_marketplaces_org_id_fkey,
    ADD CONSTRAINT plugin_marketplaces_org_id_fkey
        FOREIGN KEY (org_id) REFERENCES organizations(org_id) ON DELETE CASCADE;
