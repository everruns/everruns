-- Allow draft apps to be created before an agent or initial channel is chosen.

ALTER TABLE apps
    ALTER COLUMN agent_id DROP NOT NULL,
    ALTER COLUMN channel_type DROP NOT NULL;
