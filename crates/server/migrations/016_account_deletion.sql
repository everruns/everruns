-- Fix FK constraints that block user deletion.
-- audit_logs.actor_id and organizations.created_by default to NO ACTION (restrict).
-- Change to ON DELETE SET NULL to preserve rows while allowing user deletion.

-- audit_logs.actor_id → SET NULL on user delete
ALTER TABLE audit_logs
    DROP CONSTRAINT IF EXISTS audit_logs_actor_id_fkey,
    ADD CONSTRAINT audit_logs_actor_id_fkey
        FOREIGN KEY (actor_id) REFERENCES users(id) ON DELETE SET NULL;

-- organizations.created_by → SET NULL on user delete
ALTER TABLE organizations
    DROP CONSTRAINT IF EXISTS organizations_created_by_fkey,
    ADD CONSTRAINT organizations_created_by_fkey
        FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE SET NULL;
