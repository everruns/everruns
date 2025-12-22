-- Remove Capability CHECK Constraint (v0.2.3)
-- Decision: Remove capability_id CHECK constraint to allow dynamic capabilities
--
-- Previously capability_id was validated against a fixed list in the database.
-- Now validation happens at the application layer via CapabilityRegistry.
-- This allows adding new capabilities without database migrations.

-- ============================================
-- Drop the check constraint
-- ============================================

ALTER TABLE agent_capabilities DROP CONSTRAINT agent_capabilities_capability_id_check;
