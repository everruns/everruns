-- LLM-judge scoring for Observers. See specs/online-evals.md.
--
-- Adds the judge output (categorical label) and judge token/cost accounting to
-- trace_scores. Rule scores leave these NULL.

ALTER TABLE trace_scores
    ADD COLUMN label TEXT,
    ADD COLUMN judge_input_tokens BIGINT,
    ADD COLUMN judge_output_tokens BIGINT,
    ADD COLUMN judge_cost_usd DOUBLE PRECISION;
