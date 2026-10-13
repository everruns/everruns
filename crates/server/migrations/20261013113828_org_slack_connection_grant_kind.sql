-- org_slack_connection_grant_kind
-- Where a connected workspace's tokens came from, which decides how they are
-- refreshed: a pasted app configuration token rotates through
-- tooling.tokens.rotate, a manager app's OAuth grant through oauth.v2.access
-- with the manager app's client credentials (EVE-1102).

ALTER TABLE org_slack_connections
    ADD COLUMN grant_kind TEXT NOT NULL DEFAULT 'config_token',
    ADD CONSTRAINT org_slack_connections_grant_kind_check
        CHECK (grant_kind IN ('config_token', 'manager_app'));

COMMENT ON COLUMN org_slack_connections.grant_kind IS
    'config_token: pasted configuration token. manager_app: OAuth grant from the deployment manager app.';
