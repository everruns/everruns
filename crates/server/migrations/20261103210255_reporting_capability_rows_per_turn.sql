-- Reporting rework (knowledge/evaluation/reporting.md, "Capability rows per turn").
--
-- 1. Capability snapshot rows are keyed per turn. Every model call emits a
--    `capability.usage` event listing the turn's resolved capabilities and
--    exposed tools (about 57 records on the stress bench, 3.9 model calls per
--    turn). Keyed per event, each call wrote all of them again; keyed per
--    turn, later calls of the same turn insert nothing. Events without a turn
--    id keep the per-event key. The projector, the repair job and the
--    backfill share this one definition so they agree on what "already
--    projected" means.
-- 2. The fact tables lose their `organizations` foreign key. Every fact row
--    paid a key check on insert (about 220 per turn), and the only hard
--    delete of an organization is the rollback of a failed creation, before
--    any session or fact can exist. Reports always filter by org_id.

CREATE OR REPLACE FUNCTION reporting_capability_usage_key(
    event_id UUID,
    context JSONB,
    data JSONB,
    record JSONB,
    ordinality BIGINT
) RETURNS TEXT
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT CASE
        WHEN COALESCE(context->>'turn_id', data->>'turn_id') IS NOT NULL THEN
            'turn:' || COALESCE(context->>'turn_id', data->>'turn_id')
            || ':capability_usage:' || (record->>'usage_kind')
            || ':' || (record->>'capability_id')
            || ':' || COALESCE(record->>'tool_name', '')
        ELSE
            'event:' || event_id::TEXT || ':capability_usage:' || ordinality::TEXT
    END
$$;

ALTER TABLE fact_budget_posting DROP CONSTRAINT IF EXISTS fact_budget_posting_org_id_fkey;
ALTER TABLE fact_capability_usage DROP CONSTRAINT IF EXISTS fact_capability_usage_org_id_fkey;
ALTER TABLE fact_llm_generation DROP CONSTRAINT IF EXISTS fact_llm_generation_org_id_fkey;
ALTER TABLE fact_session DROP CONSTRAINT IF EXISTS fact_session_org_id_fkey;
ALTER TABLE fact_tool_call DROP CONSTRAINT IF EXISTS fact_tool_call_org_id_fkey;
ALTER TABLE fact_turn DROP CONSTRAINT IF EXISTS fact_turn_org_id_fkey;
