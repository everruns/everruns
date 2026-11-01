-- eval_run_budget
-- A run can carry a spending cap. The cap itself is an ordinary budget with
-- subject `eval_run` (keyed by the run's public id), so the loop gate stops a
-- case's turn when the run's money is spent and grading cost debits the same
-- pool. This column only records what was asked for, for display.
ALTER TABLE eval_runs ADD COLUMN max_cost_usd DOUBLE PRECISION;

ALTER TABLE public.budgets DROP CONSTRAINT budgets_subject_type_check;
ALTER TABLE public.budgets ADD CONSTRAINT budgets_subject_type_check CHECK (
    subject_type = ANY (ARRAY[
        'session'::text, 'agent'::text, 'user'::text, 'org'::text,
        'agent_trigger'::text, 'agent_channel'::text,
        'session_schedule'::text, 'org_unattended'::text,
        'eval_run'::text
    ])
);
