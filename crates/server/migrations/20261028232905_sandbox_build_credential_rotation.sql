-- Register the temporary binding with the normal encrypted-column rotation path.
ALTER TABLE sandbox_template_builds ADD COLUMN credential_snapshot_encrypted bytea;
UPDATE sandbox_template_builds SET credential_snapshot_encrypted = credential_snapshot;
ALTER TABLE sandbox_template_builds DROP COLUMN credential_snapshot;
