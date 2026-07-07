-- Everruns as a host/viewer for externally-executed eval results (e.g. Mira).
-- See proposals/mira-results-publishing.md.
--
-- An external run is ingested already-complete: everruns never spawns sessions
-- for it. These columns let a run carry provenance and grouping without a new
-- entity (we extend EvalRun rather than fork the model):
--
--   source         - 'internal' (everruns executed it) or 'external' (imported).
--                    Defaults to 'internal' so every existing run is unchanged.
--   source_run_id  - the external system's stable run id. One external run maps
--                    to one EvalRun per eval, all sharing this id; it is both the
--                    cross-eval group key and the idempotency key (re-publishing
--                    the same id replaces the prior run for that eval).
--   attribution    - open-vocab JSON provenance (system, version, url, env
--                    labels). JSONB, not columns, so new fields need no schema
--                    change — matching how per-result transcript/metrics ride in
--                    the existing eval_case_results.metadata envelope.
ALTER TABLE eval_runs
    ADD COLUMN source TEXT NOT NULL DEFAULT 'internal',
    ADD COLUMN source_run_id TEXT,
    ADD COLUMN attribution JSONB;

-- Grouping + idempotency lookups are always org-scoped by source_run_id.
CREATE INDEX idx_eval_runs_source_run_id
    ON eval_runs (org_id, source_run_id)
    WHERE source_run_id IS NOT NULL;

-- External systems may report a case as never executed ('skipped': model
-- unavailable, filtered out). Widen the case-result status CHECK to allow it.
ALTER TABLE eval_case_results DROP CONSTRAINT eval_case_results_status_check;
ALTER TABLE eval_case_results ADD CONSTRAINT eval_case_results_status_check
    CHECK (status IN ('pending', 'running', 'passed', 'failed', 'errored', 'timeout', 'skipped'));
