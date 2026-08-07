-- Repeated requests for the same run and export options reuse one durable
-- artifact instead of enqueueing and persisting duplicate work.
WITH duplicates AS (
    SELECT id,
           ROW_NUMBER() OVER (
               PARTITION BY org_id, eval_run_id, request
               ORDER BY created_at, id
           ) AS duplicate_number
    FROM eval_run_datasets
    WHERE eval_run_id IS NOT NULL
)
DELETE FROM eval_run_datasets
WHERE id IN (SELECT id FROM duplicates WHERE duplicate_number > 1);

CREATE UNIQUE INDEX idx_eval_run_datasets_request
    ON eval_run_datasets(org_id, eval_run_id, request)
    WHERE eval_run_id IS NOT NULL;
