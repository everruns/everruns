-- NULL preserves the existing inherited fallback until Chat is configured independently.
ALTER TABLE organization_settings ADD COLUMN chat_model_id UUID;
ALTER TABLE organization_settings ADD CONSTRAINT organization_settings_chat_model_org_fkey
    FOREIGN KEY (org_id, chat_model_id) REFERENCES models (org_id, id)
    ON DELETE SET NULL (chat_model_id);
