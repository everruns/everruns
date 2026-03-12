-- Workflow Snapshots for Replay Checkpointing (EVE-86)
--
-- Periodic snapshots of serialized workflow state to avoid replaying
-- all events from the beginning. On replay, the engine loads the latest
-- snapshot + events since that snapshot's sequence number.
--
-- Reduces replay cost from O(total_events) to O(checkpoint_interval).

CREATE TABLE IF NOT EXISTS durable_workflow_snapshots (
    id BIGSERIAL PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    sequence_num INT NOT NULL,         -- Event sequence at which snapshot was taken
    snapshot_data BYTEA NOT NULL,      -- Serialized workflow state (JSON)
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Only one snapshot per (workflow, sequence) pair
    UNIQUE(workflow_id, sequence_num)
);

-- Fast lookup: latest snapshot for a workflow
CREATE INDEX IF NOT EXISTS idx_durable_workflow_snapshots_latest
    ON durable_workflow_snapshots(workflow_id, sequence_num DESC);
