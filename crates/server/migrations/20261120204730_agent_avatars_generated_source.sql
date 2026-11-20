-- generate_agent_avatar stores the picture an image model drew as source
-- 'generated', next to 'upload' and the curated 'preset:<id>' sources.
ALTER TABLE agent_avatars DROP CONSTRAINT agent_avatars_source_check;
ALTER TABLE agent_avatars ADD CONSTRAINT agent_avatars_source_check
    CHECK (source IN ('upload', 'generated') OR source ~ '^preset:[a-z]+-[a-z]+$');
