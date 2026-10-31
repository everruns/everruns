-- A build's provider authority must not silently move when its named connection
-- is rotated or replaced. This encrypted, temporary binding is never serialized
-- into the public build record and is cleared after apply or confirmed cleanup.
ALTER TABLE sandbox_template_builds ADD COLUMN credential_snapshot bytea;
