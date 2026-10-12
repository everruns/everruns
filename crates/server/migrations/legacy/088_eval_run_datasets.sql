-- Async dataset-export handles (specs/dataset-export.md, Phase 2).
--
-- Phase 1 exported a completed eval run to reward-labeled NDJSON synchronously
-- from `POST .../dataset`. Phase 2 makes the export a handle: the POST enqueues
-- a background export job (mirroring the fire-and-forget eval runner) and
-- returns a dataset handle; `GET .../dataset/{dataset_id}` polls status and,
-- once completed, returns the stored NDJSON body.
--
-- The whole export — request parameters, status, and the produced NDJSON — is
-- stored inline on a single row, keyed by the source eval run and org. This
-- mirrors `agent_health_check_runs`: a disposable, system-generated artifact
-- with no separate child rows. `body` holds the produced NDJSON so re-fetching
-- is cheap and the export is a durable derived artifact.

CREATE TABLE eval_run_datasets (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL,
    public_id TEXT NOT NULL,
    -- The eval run this dataset was exported from. Nullable so the dataset
    -- handle survives run deletion for audit; still org-scoped.
    eval_run_id UUID,
    -- The export request (format + filters + redaction) captured verbatim so a
    -- handle records exactly which view of the run it represents.
    request JSONB NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed')),
    -- Produced NDJSON, one record per surviving case. Set on completion.
    body TEXT,
    -- Number of NDJSON records written (surviving cases). Set on completion.
    record_count BIGINT,
    error_message TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(org_id, public_id),
    CONSTRAINT eval_run_datasets_public_id_format
        CHECK (public_id ~ '^evaldataset_[0-9a-f]{32}$'),
    CONSTRAINT eval_run_datasets_org_id_fk
        FOREIGN KEY (org_id) REFERENCES organizations(org_id),
    CONSTRAINT eval_run_datasets_eval_run_id_fk
        FOREIGN KEY (eval_run_id) REFERENCES eval_runs(id) ON DELETE SET NULL
);

CREATE INDEX idx_eval_run_datasets_org_id ON eval_run_datasets(org_id);
-- List a run's datasets newest-first.
CREATE INDEX idx_eval_run_datasets_run
    ON eval_run_datasets(eval_run_id, created_at DESC);

CREATE TRIGGER update_eval_run_datasets_updated_at
    BEFORE UPDATE ON eval_run_datasets
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
