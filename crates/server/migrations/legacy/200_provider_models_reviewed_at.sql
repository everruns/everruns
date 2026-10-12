-- When someone last reviewed this provider's discovered models. A discovered,
-- still-disabled model created after this moment is "new": the Models page
-- offers it for review instead of leaving it buried among hundreds of rows.
-- NULL means never reviewed, so everything a fresh provider discovers is new.
ALTER TABLE providers ADD COLUMN models_reviewed_at TIMESTAMPTZ;

-- Existing catalogs were curated on the old page; start them reviewed so the
-- first deploy does not flag every model ever discovered.
UPDATE providers SET models_reviewed_at = NOW();
