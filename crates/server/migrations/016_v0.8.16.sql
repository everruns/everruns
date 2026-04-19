-- v0.8.16: eval case result metadata and artifacts
-- Squashed from: 016_eval_case_result_metadata.sql, 017_eval_artifacts.sql

-- ============================================
-- from 016_eval_case_result_metadata.sql
-- ============================================

ALTER TABLE eval_case_results
ADD COLUMN metadata JSONB;

-- ============================================
-- from 017_eval_artifacts.sql
-- ============================================
-- Add artifact specs to eval cases and collected artifact payloads to eval case results.

ALTER TABLE eval_cases
    ADD COLUMN artifacts JSONB;

ALTER TABLE eval_case_results
    ADD COLUMN artifacts JSONB;
