-- Migration 010: Connection provider metadata
-- Adds provider_metadata JSONB column to user_connections and
-- agent_identity_connections for storing provider-specific data
-- (e.g. Deno org slug for personal tokens).

ALTER TABLE user_connections
    ADD COLUMN provider_metadata JSONB;

ALTER TABLE agent_identity_connections
    ADD COLUMN provider_metadata JSONB;
