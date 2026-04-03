-- Network access list: per-harness/agent/session URL allowlist and blocklist.
-- See specs/network-access.md for design.
-- Merge semantics: allowed=intersect, blocked=union across layers.

ALTER TABLE harnesses
ADD COLUMN network_access JSONB;

ALTER TABLE agents
ADD COLUMN network_access JSONB;

ALTER TABLE sessions
ADD COLUMN network_access JSONB;
