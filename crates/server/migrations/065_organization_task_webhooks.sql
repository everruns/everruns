-- Organization task webhooks: outbound HTTP targets notified when a session
-- task reaches a terminal state (Succeeded / Failed / Canceled).
--
-- Each webhook can be independently enabled/disabled. The secret is used to
-- sign the payload with HMAC-SHA256 so receivers can verify the origin.
-- See specs/session-tasks.md and EVE-579.

CREATE TABLE organization_task_webhooks (
    id          BIGSERIAL PRIMARY KEY,
    public_id   TEXT NOT NULL UNIQUE,
    org_id      BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    url         TEXT NOT NULL,
    secret      TEXT,
    enabled     BOOLEAN NOT NULL DEFAULT TRUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX organization_task_webhooks_org_id_idx
    ON organization_task_webhooks (org_id);
