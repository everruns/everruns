-- Enforce the Workspace Volume name semantics used by the API and in-memory
-- backend: non-deleted volume names are unique per org regardless of case.

CREATE UNIQUE INDEX idx_volumes_org_lower_name_active
    ON volumes(org_id, LOWER(name))
    WHERE status != 'deleted';
