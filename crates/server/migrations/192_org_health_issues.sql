-- Organization-level health issues, which belong to no channel.
--
-- The first one is the active-turn limit (ORG_MAX_ACTIVE_TURNS): org members
-- see in Settings -> Health that new messages are being refused. A channel
-- issue stays unique per (org, channel, code); an org issue is unique per
-- (org, code), so it reopens as a new episode instead of piling up rows.

ALTER TABLE health_issues
    ALTER COLUMN channel_id DROP NOT NULL,
    ALTER COLUMN channel_revision DROP NOT NULL;

ALTER TABLE health_issues DROP CONSTRAINT health_issues_org_id_channel_id_code_key;

CREATE UNIQUE INDEX health_issues_channel_code
    ON health_issues(org_id, channel_id, code) WHERE channel_id IS NOT NULL;
CREATE UNIQUE INDEX health_issues_org_code
    ON health_issues(org_id, code) WHERE channel_id IS NULL;
