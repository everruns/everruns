-- Agent Plugins v1 names may contain 64 characters. With the `plugin:`
-- namespace, persisted capability references therefore require 71 characters.

ALTER TABLE agent_capabilities
    ALTER COLUMN capability_id TYPE VARCHAR(71);

ALTER TABLE harness_capabilities
    ALTER COLUMN capability_id TYPE VARCHAR(71);

ALTER TABLE plugin_installs
    DROP CONSTRAINT plugin_installs_name_format,
    DROP CONSTRAINT plugin_installs_name_length,
    ALTER COLUMN name TYPE VARCHAR(64),
    ADD CONSTRAINT plugin_installs_name_format
        CHECK (name ~ '^([a-z0-9]|[a-z0-9][a-z0-9._-]*[a-z0-9])$'),
    ADD CONSTRAINT plugin_installs_name_length
        CHECK (octet_length(name) <= 64);
