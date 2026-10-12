-- Automatic Agent draft snapshots live beside user-published versions.

ALTER TABLE agent_versions
    ADD COLUMN is_published BOOLEAN NOT NULL DEFAULT TRUE;

ALTER TABLE agent_versions
    DROP CONSTRAINT agent_versions_change_kind_check;

ALTER TABLE agent_versions
    ADD CONSTRAINT agent_versions_change_kind_check
        CHECK (change_kind IN ('auto', 'manual', 'patch', 'minor', 'major', 'import', 'rollback', 'fork'));

CREATE INDEX idx_agent_versions_published
    ON agent_versions(org_id, agent_id, version_number DESC)
    WHERE is_published;

COMMENT ON COLUMN agent_versions.is_published IS
    'True for user-published semver versions; false for automatic draft snapshots.';
