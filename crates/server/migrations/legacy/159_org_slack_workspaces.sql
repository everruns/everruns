-- An organization can connect more than one Slack workspace, and each
-- connection records which workspace it is.
--
-- team_id is nullable only for rows created before this migration: Slack's
-- tooling.tokens.rotate response carries it, and the rotation sweep rotates
-- rows missing it on its next pass, so they fill in without a reconnect.
ALTER TABLE org_slack_connections
    ADD COLUMN team_id TEXT,
    ADD COLUMN team_name TEXT;

ALTER TABLE org_slack_connections
    DROP CONSTRAINT org_slack_connections_org_id_key;

ALTER TABLE org_slack_connections
    ADD CONSTRAINT org_slack_connections_org_team_key UNIQUE (org_id, team_id);
