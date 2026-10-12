-- EVE-509: composite index to speed up per-org active eval run count queries
-- (SELECT COUNT(*) WHERE org_id = $1 AND status IN ('pending', 'running'))
CREATE INDEX idx_eval_runs_org_id_status ON eval_runs(org_id, status);
