-- Datasets and experiments (Evals rebuild, step 3;
-- knowledge/evaluation/experiments.md).
--
-- An eval's cases are its dataset. `evals.version` counts changes to them so a
-- run records which version it ran. Triggers keep it, because cases change
-- through several paths (authoring, imports, ATIF) and a missed bump would
-- silently compare runs of different datasets as if they were the same.

ALTER TABLE evals ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

CREATE FUNCTION bump_eval_version() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    UPDATE evals SET version = version + 1
    WHERE id = COALESCE(NEW.eval_id, OLD.eval_id);
    RETURN NULL;
END;
$$;

CREATE TRIGGER eval_cases_bump_version_insert
    AFTER INSERT ON eval_cases FOR EACH ROW EXECUTE FUNCTION bump_eval_version();
CREATE TRIGGER eval_cases_bump_version_update
    AFTER UPDATE ON eval_cases FOR EACH ROW EXECUTE FUNCTION bump_eval_version();
CREATE TRIGGER eval_cases_bump_version_delete
    AFTER DELETE ON eval_cases FOR EACH ROW EXECUTE FUNCTION bump_eval_version();

-- One experiment runs one dataset version against several variants; each
-- variant is an ordinary run tagged with the experiment and its label.
CREATE TABLE eval_experiments (
    id UUID PRIMARY KEY,
    public_id TEXT NOT NULL UNIQUE
        CONSTRAINT eval_experiments_public_id_format CHECK (public_id ~ '^evalexp_[0-9a-f]{32}$'),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    eval_id UUID NOT NULL REFERENCES evals(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    dataset_version INTEGER NOT NULL,
    trials INTEGER NOT NULL,
    max_cost_usd DOUBLE PRECISION,
    -- The label results are compared against: the first variant.
    baseline_label TEXT NOT NULL,
    created_by_user_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX eval_experiments_eval_idx ON eval_experiments (eval_id, created_at DESC);
CREATE INDEX eval_experiments_org_idx ON eval_experiments (org_id, created_at DESC);

ALTER TABLE eval_runs
    ADD COLUMN dataset_version INTEGER,
    ADD COLUMN experiment_id UUID REFERENCES eval_experiments(id) ON DELETE SET NULL,
    ADD COLUMN variant_label TEXT;

CREATE INDEX eval_runs_experiment_idx ON eval_runs (experiment_id)
    WHERE experiment_id IS NOT NULL;

-- Every new run records the dataset version it was created from.
CREATE FUNCTION set_eval_run_dataset_version() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.dataset_version IS NULL THEN
        SELECT version INTO NEW.dataset_version FROM evals WHERE id = NEW.eval_id;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER eval_runs_set_dataset_version
    BEFORE INSERT ON eval_runs FOR EACH ROW EXECUTE FUNCTION set_eval_run_dataset_version();
