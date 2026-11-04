-- Limits on unattended background work (knowledge/operations/background-limits.md).
--
-- A test agent's session schedule fired every 5 minutes for 13 days and spent
-- $106 with nothing to stop it. Schedules an agent creates now carry an end
-- (ends_at and/or max_runs), the agent's own stop condition, and the reason
-- they stopped on their own. Each schedule gets a `session_schedule` budget,
-- and each org an `org_unattended` daily budget.

ALTER TABLE public.session_schedules
    ADD COLUMN ends_at TIMESTAMPTZ,
    ADD COLUMN max_runs INTEGER,
    ADD COLUMN stop_when TEXT,
    ADD COLUMN stop_reason TEXT,
    ADD CONSTRAINT session_schedules_max_runs_check CHECK (max_runs IS NULL OR max_runs >= 1);

-- The schedule whose fire produced this input message, so metering can
-- charge the turn's generations to the schedule's budget.
ALTER TABLE public.runtime_invocations
    ADD COLUMN schedule_id UUID;

-- Org overrides of the code defaults, edited in Settings → Limits. NULL means
-- every default applies.
ALTER TABLE public.organization_settings
    ADD COLUMN background_limits JSONB;

ALTER TABLE public.budgets DROP CONSTRAINT budgets_subject_type_check;
ALTER TABLE public.budgets ADD CONSTRAINT budgets_subject_type_check CHECK (
    subject_type = ANY (ARRAY[
        'session'::text, 'agent'::text, 'user'::text, 'org'::text,
        'agent_trigger'::text, 'agent_channel'::text,
        'session_schedule'::text, 'org_unattended'::text
    ])
);

-- One live unattended budget per org; it is created lazily on first
-- unattended spend and edited through Settings → Limits.
CREATE UNIQUE INDEX budgets_org_unattended_unique
    ON public.budgets (org_id)
    WHERE subject_type = 'org_unattended' AND status <> 'disabled';
