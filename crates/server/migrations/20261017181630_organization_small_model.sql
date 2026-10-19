-- Shared small chat model, independent of the main and Decisions defaults.
-- Disabled selections stay pinned; deleting the model clears this selection.
ALTER TABLE organization_settings
    ADD COLUMN small_model_id UUID REFERENCES models(id) ON DELETE SET NULL;
