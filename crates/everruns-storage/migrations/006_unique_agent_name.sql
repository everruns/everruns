-- Add unique constraint on agent name to ensure idempotent seeding
-- This prevents duplicate agents with the same name

ALTER TABLE agents ADD CONSTRAINT agents_name_unique UNIQUE (name);
