-- Detection for unattended background work (knowledge/operations/background-limits.md,
-- "Detection").
--
-- A schedule remembers a fingerprint of how its last finished run ended and
-- how many runs in a row ended that way, so the scheduler can warn the agent
-- after 24 identical runs and stop the schedule after 48.
ALTER TABLE public.session_schedules
    ADD COLUMN outcome_fingerprint TEXT,
    ADD COLUMN same_outcome_runs INTEGER NOT NULL DEFAULT 0,
    -- The fire whose outcome was last counted, so a run is never counted twice.
    ADD COLUMN outcome_input_message_id UUID;

-- Health issues about one thing that is neither a channel nor the whole org:
-- a schedule (`sched_…`), an agent (`agent_…`) or a session (`session_…`).
-- `agent_id` names the agent for issues that have no channel; `evidence`
-- holds the numbers the issue's copy is built from.
ALTER TABLE public.health_issues
    ADD COLUMN subject_id TEXT,
    ADD COLUMN agent_id UUID REFERENCES public.agents(id) ON DELETE CASCADE,
    ADD COLUMN session_id UUID,
    ADD COLUMN evidence JSONB;

DROP INDEX public.health_issues_org_code;
CREATE UNIQUE INDEX health_issues_org_code
    ON public.health_issues (org_id, code)
    WHERE channel_id IS NULL AND subject_id IS NULL;
CREATE UNIQUE INDEX health_issues_subject_code
    ON public.health_issues (org_id, code, subject_id)
    WHERE subject_id IS NOT NULL;
