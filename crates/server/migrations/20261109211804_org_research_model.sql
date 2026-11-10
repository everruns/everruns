-- Organization research model: the strongest model for hard reasoning and
-- long research. NULL means Automatic, resolved when used (Anthropic's
-- recommended research model, then OpenAI's, then any other provider's, then
-- the org default). Deleting the model falls back to Automatic.
ALTER TABLE organization_settings
    ADD COLUMN research_model_id UUID REFERENCES models(id) ON DELETE SET NULL;

-- An agent can follow an organization model role instead of pinning a model.
-- Mutually exclusive with default_model_id; the server keeps one of them NULL.
ALTER TABLE agents
    ADD COLUMN model_role TEXT CHECK (model_role IN ('research'));
