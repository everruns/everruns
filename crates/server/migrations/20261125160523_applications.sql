-- Applications are audience/publication containers, independent of the retired App.
CREATE TABLE applications (
    id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    slug TEXT NOT NULL UNIQUE CHECK (slug ~ '^[a-z][a-z0-9-]{1,46}[a-z0-9]$'),
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','published','archived')),
    config JSONB NOT NULL,
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, id)
);
CREATE INDEX applications_org ON applications (org_id, created_at DESC);
-- Named resource bindings are exclusive: another audience cannot acquire the
-- same chat or room. Memory sharing is deliberate and has an explicit access mode.
CREATE TABLE application_resources (
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    slug TEXT NOT NULL,
    name TEXT NOT NULL,
    position INTEGER NOT NULL,
    channel_id UUID UNIQUE REFERENCES agent_channels(id) ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED,
    room_id UUID UNIQUE REFERENCES rooms(id) ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED,
    memory_id UUID REFERENCES memories(id) ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED,
    writable BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (application_id, slug),
    CHECK (num_nonnulls(channel_id,room_id,memory_id) = 1),
    CHECK (NOT writable OR memory_id IS NOT NULL)
);
CREATE INDEX application_resources_memory ON application_resources(memory_id) WHERE memory_id IS NOT NULL;
CREATE TABLE application_room_members (
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    virtual_user_id UUID NOT NULL REFERENCES virtual_users(id) ON DELETE CASCADE,
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    member_id UUID NOT NULL REFERENCES room_members(id) ON DELETE CASCADE,
    PRIMARY KEY (application_id,virtual_user_id,room_id),
    UNIQUE (member_id)
);
