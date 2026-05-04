-- Session stats indexes
--
-- Entity detail stats aggregate sessions by org + harness/agent and join turn
-- lifecycle events by session. Add compound indexes for bounded detail queries.

CREATE INDEX idx_sessions_org_agent_id ON sessions(org_id, agent_id);
CREATE INDEX idx_sessions_org_harness_id ON sessions(org_id, harness_id);
