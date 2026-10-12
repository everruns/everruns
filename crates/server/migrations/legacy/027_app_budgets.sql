-- App / channel scoped budgets and periodic reset support.
--
-- Behind the experimental `app_budgets` feature flag (FEATURE_APP_BUDGETS).
-- See specs/budgeting.md for the design intent. The feature flag gates the
-- API surface; the schema additions are unconditional so existing rows
-- continue to work and the flag can be flipped without further migrations.
--
-- Changes:
--   1. Allow `app` and `app_channel` as `subject_type` values on `budgets`.
--   2. Add `period_started_at TIMESTAMPTZ` so the budget service can detect
--      and execute period rollovers (Duration / Rolling / Calendar periods).

ALTER TABLE budgets
    DROP CONSTRAINT IF EXISTS budgets_subject_type_check;

ALTER TABLE budgets
    ADD CONSTRAINT budgets_subject_type_check
    CHECK (subject_type IN ('session', 'agent', 'user', 'org', 'app', 'app_channel'));

ALTER TABLE budgets
    ADD COLUMN IF NOT EXISTS period_started_at TIMESTAMPTZ;

-- Backfill: budgets with a configured period that haven't started a window
-- yet (e.g. existing rolling budgets) start the window now so the next
-- evaluation has a stable reference point.
UPDATE budgets
   SET period_started_at = NOW()
 WHERE period IS NOT NULL
   AND period_started_at IS NULL;
