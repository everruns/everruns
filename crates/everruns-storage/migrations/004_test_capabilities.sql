-- Test Capabilities Schema (v0.2.2)
-- Decision: Add test_math and test_weather capabilities for tool calling smoke tests

-- ============================================
-- Update capability_id check constraint
-- ============================================

-- Drop the old constraint and add a new one with the test capabilities
ALTER TABLE agent_capabilities DROP CONSTRAINT agent_capabilities_capability_id_check;

ALTER TABLE agent_capabilities ADD CONSTRAINT agent_capabilities_capability_id_check
    CHECK (capability_id IN ('noop', 'current_time', 'research', 'sandbox', 'file_system', 'test_math', 'test_weather'));
