-- entity_localizations
-- Display overlays stay separate from stable addresses and execution configuration.
ALTER TABLE agents ADD COLUMN localizations JSONB NOT NULL DEFAULT '{}'
    CHECK (jsonb_typeof(localizations) = 'object');
ALTER TABLE harnesses ADD COLUMN localizations JSONB NOT NULL DEFAULT '{}'
    CHECK (jsonb_typeof(localizations) = 'object');
