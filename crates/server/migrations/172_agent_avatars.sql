-- Agent avatars: an uploaded image, square-cropped once at upload, plus
-- pre-rendered square and circular PNG presets served from a public,
-- immutable URL. A replacement gets a fresh avatar id, so every URL can be
-- cached forever. `source` leaves room for a curated predefined set later.
CREATE TABLE agent_avatars (
    id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    source TEXT NOT NULL DEFAULT 'upload' CHECK (source IN ('upload')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX agent_avatars_agent_idx ON agent_avatars (agent_id);

CREATE TABLE agent_avatar_variants (
    avatar_id UUID NOT NULL REFERENCES agent_avatars(id) ON DELETE CASCADE,
    variant TEXT NOT NULL,
    content_type TEXT NOT NULL,
    data BYTEA NOT NULL,
    PRIMARY KEY (avatar_id, variant)
);

ALTER TABLE agents
    ADD COLUMN avatar_id UUID NULL REFERENCES agent_avatars(id) ON DELETE SET NULL;
