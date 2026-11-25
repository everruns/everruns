-- A person's own plugin choices: which of the organization's approved plugins
-- they add for themselves, or turn off, in Work Chat (where the channel allows
-- it) and on Settings > Personal > Plugins.
--
-- No row means the person has not chosen: the agent's plugins apply as the
-- agent was set up. `enabled` true adds the plugin to their new
-- conversations; false keeps it out, including one the agent comes with.
-- Rows are updated in place, never deleted for a change of mind. They go away
-- with the install they name, which is the org's own decision.

CREATE TABLE plugin_choices (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    principal_id UUID NOT NULL REFERENCES principals(id),
    plugin_install_id UUID NOT NULL REFERENCES plugin_installs(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT plugin_choices_one_per_person UNIQUE (org_id, principal_id, plugin_install_id)
);

CREATE INDEX plugin_choices_principal ON plugin_choices (org_id, principal_id);
