-- Retain the stable curated ID alongside the existing avatar variants.
ALTER TABLE agent_avatars DROP CONSTRAINT agent_avatars_source_check;
ALTER TABLE agent_avatars ADD CONSTRAINT agent_avatars_source_check
    CHECK (source = 'upload' OR source ~ '^preset:[a-z]+-[a-z]+$');
