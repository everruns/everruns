-- Add post messages column to eval_cases for verification prompts after conversation completes.
-- Used by benchmarks like SWE-bench where scoring requires running commands in the sandbox
-- after the agent applies a fix.

ALTER TABLE eval_cases ADD COLUMN IF NOT EXISTS post JSONB;
