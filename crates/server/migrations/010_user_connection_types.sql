-- Add connection_type to user_connections to distinguish OAuth vs API key connections.
-- Existing rows default to 'oauth'. New API-key connections (e.g. Daytona) use 'api_key'.

ALTER TABLE user_connections
  ADD COLUMN connection_type VARCHAR(20) NOT NULL DEFAULT 'oauth';
