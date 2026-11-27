-- Old default manual fires used the schedule ingress label. A stored schedule
-- label alone cannot establish automatic attribution for those deliveries.
-- Only the corrected ingress writer marks a source as version 1.
ALTER TABLE agent_trigger_deliveries
    ADD COLUMN source_version INTEGER NOT NULL DEFAULT 0;
