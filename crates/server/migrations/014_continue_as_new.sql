-- Add continued_as_new_id column to durable_workflow_instances
-- Tracks the new workflow ID when a workflow rolls over via continue-as-new
ALTER TABLE durable_workflow_instances
ADD COLUMN IF NOT EXISTS continued_as_new_id UUID REFERENCES durable_workflow_instances(id);

-- Index for looking up continuation chains
CREATE INDEX IF NOT EXISTS idx_durable_workflow_instances_continued_as_new_id
ON durable_workflow_instances(continued_as_new_id)
WHERE continued_as_new_id IS NOT NULL;
