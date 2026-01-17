-- Migration: Remove usage tracking trigger, move logic to Rust
--
-- Rationale (see specs/architecture.md):
-- 1. Triggers are invisible to application code and hard to debug
-- 2. Triggers don't work in DEV_MODE (in-memory storage)
-- 3. Triggers create hidden coupling between tables
-- 4. All primary keys should use uuidv7() for consistency
--
-- This migration:
-- 1. Drops the trigger that auto-populates llm_generations
-- 2. Drops the trigger function
-- 3. Changes llm_generations.id default to uuidv7()
--
-- The logic is now implemented in Rust via UsageTrackingListener (EventListener pattern).

-- ============================================================================
-- 1. Drop the trigger
-- ============================================================================

DROP TRIGGER IF EXISTS trigger_process_llm_generation ON events;

-- ============================================================================
-- 2. Drop the trigger function
-- ============================================================================

DROP FUNCTION IF EXISTS process_llm_generation_event();

-- ============================================================================
-- 3. Change llm_generations.id default to uuidv7()
-- ============================================================================

ALTER TABLE llm_generations ALTER COLUMN id SET DEFAULT uuidv7();
