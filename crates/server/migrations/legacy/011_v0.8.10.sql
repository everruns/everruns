-- v0.8.10: Eval target refactor + post messages
-- Squashed from: 011_evals_target.sql, 012_evals_post.sql

-- ============================================
-- 011_evals_target: replace agent_id/harness_id with target JSONB
-- ============================================

-- Migrate existing data into target column
ALTER TABLE evals ADD COLUMN target JSONB;

UPDATE evals SET target = jsonb_build_object(
    'type', 'session',
    'harness_id', (SELECT public_id FROM harnesses WHERE harnesses.id = evals.harness_id),
    'agent_id', (SELECT public_id FROM agents WHERE agents.id = evals.agent_id)
) WHERE agent_id IS NOT NULL AND harness_id IS NOT NULL;

-- Drop old FK constraints and columns
ALTER TABLE evals DROP CONSTRAINT IF EXISTS evals_agent_id_fk;
ALTER TABLE evals DROP CONSTRAINT IF EXISTS evals_harness_id_fk;
ALTER TABLE evals DROP COLUMN agent_id;
ALTER TABLE evals DROP COLUMN harness_id;

-- ============================================
-- eval_cases: add target JSONB
-- ============================================

ALTER TABLE eval_cases ADD COLUMN target JSONB;

-- ============================================
-- eval_runs: add target JSONB
-- ============================================

ALTER TABLE eval_runs ADD COLUMN target JSONB;

-- ============================================
-- eval_case_results: add target + target_snapshot JSONB
-- ============================================

ALTER TABLE eval_case_results ADD COLUMN target JSONB;
ALTER TABLE eval_case_results ADD COLUMN target_snapshot JSONB;

-- ============================================
-- 012_evals_post: add post messages column to eval_cases
-- ============================================

ALTER TABLE eval_cases ADD COLUMN IF NOT EXISTS post JSONB;
