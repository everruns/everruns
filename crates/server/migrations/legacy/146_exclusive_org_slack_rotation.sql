ALTER TABLE org_slack_connections
    DROP CONSTRAINT org_slack_connections_state_check;

ALTER TABLE org_slack_connections
    ADD CONSTRAINT org_slack_connections_state_check
    CHECK (state IN ('connected', 'rotating', 'reconnect_required'));

ALTER TABLE org_slack_connections
    DROP CONSTRAINT org_slack_connections_connected_tokens;

ALTER TABLE org_slack_connections
    ADD CONSTRAINT org_slack_connections_connected_tokens CHECK (
        state NOT IN ('connected', 'rotating')
        OR (
            access_token_encrypted IS NOT NULL
            AND refresh_token_encrypted IS NOT NULL
            AND access_token_expires_at IS NOT NULL
        )
    );
