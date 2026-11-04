-- Eval runs repeat each case `trials` times (pass@k / pass^k). Each repetition
-- is its own result row, numbered by `trial` from 0. Existing runs and results
-- are single-trial, which the defaults record without rewriting any row.
ALTER TABLE eval_runs ADD COLUMN trials INTEGER NOT NULL DEFAULT 1 CHECK (trials BETWEEN 1 AND 20);
ALTER TABLE eval_case_results ADD COLUMN trial INTEGER NOT NULL DEFAULT 0 CHECK (trial >= 0);
