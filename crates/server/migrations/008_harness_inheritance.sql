-- Harness inheritance
--
-- Decision: schema-only migration; existing data is upgraded by startup reconciliation and
-- service-layer logic, not by in-migration UPDATE statements.

ALTER TABLE harnesses
ADD COLUMN parent_harness_id UUID REFERENCES harnesses(id) ON DELETE RESTRICT;

CREATE INDEX idx_harnesses_parent_harness_id ON harnesses(parent_harness_id);
