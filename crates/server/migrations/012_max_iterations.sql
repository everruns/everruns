-- Add configurable max_iterations per agent and session (EVE-211)
--
-- Resolution priority: session override > agent config > system default (500)
-- NULL means "use next level in the chain"

ALTER TABLE agents ADD COLUMN max_iterations INTEGER;
ALTER TABLE sessions ADD COLUMN max_iterations INTEGER;
