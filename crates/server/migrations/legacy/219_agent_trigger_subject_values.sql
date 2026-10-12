-- Last value seen per trigger, filter path and event subject, for the
-- "only when it changes" trigger filter (`changes_only`). A watcher keyed on a
-- CI outcome wakes when `owner/repo@main:CI` flips from success to failure and
-- back, not on every result. Rows go with their trigger; the pipeline bounds
-- them per trigger (see domains/agent_triggers/events.rs).

CREATE TABLE agent_trigger_subject_values (
    trigger_id UUID NOT NULL REFERENCES agent_triggers(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    subject TEXT NOT NULL,
    value JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (trigger_id, path, subject)
);

CREATE INDEX idx_agent_trigger_subject_values_trigger_updated
    ON agent_trigger_subject_values (trigger_id, updated_at DESC);
