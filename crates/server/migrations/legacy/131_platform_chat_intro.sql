-- Platform Chat intro content for harnesses and agents.
ALTER TABLE harnesses
    ADD COLUMN intro_markdown TEXT NULL,
    ADD COLUMN short_description TEXT NULL,
    ADD COLUMN starters JSONB NULL DEFAULT '[]'::jsonb;

ALTER TABLE agents
    ADD COLUMN intro_markdown TEXT NULL,
    ADD COLUMN short_description TEXT NULL,
    ADD COLUMN starters JSONB NULL DEFAULT '[]'::jsonb;
