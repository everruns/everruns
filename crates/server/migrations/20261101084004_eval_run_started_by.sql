-- eval_run_started_by
-- Eval cases run with the authority of whoever started the run, not as the
-- internal system caller. A run started by a person records them, and each
-- case resolves their current membership, so a removed member's run stops.
-- A run started with a credential that has no person records the role it had.
-- Older runs have neither and keep running as the system caller.
ALTER TABLE eval_runs ADD COLUMN started_by_user_id UUID;
ALTER TABLE eval_runs ADD COLUMN started_by_role TEXT;
