-- Remove the Reports feature: the reporting outbox, saved reports, the repair
-- cursor and the derived fact tables. Requested by the owner, who asked for
-- the data to go with the feature; every fact row was a projection of
-- canonical rows (`events`, `sessions`, `llm_generations`, `usage_ledger`),
-- which this migration leaves untouched.
--
-- Kept on purpose:
-- - `idx_events_reporting_repair`, a partial index on `events` that hosted
--   billing also scans for `tool.completed` windows.
-- - `sessions.turn_count` / `tool_call_count` and their delete trigger: evals
--   read the counters.

DROP TABLE IF EXISTS reporting_outbox;
DROP TABLE IF EXISTS reporting_saved_reports;
DROP TABLE IF EXISTS reporting_event_repair_cursor;

DROP TABLE IF EXISTS fact_budget_posting;
DROP TABLE IF EXISTS fact_capability_usage;
DROP TABLE IF EXISTS fact_llm_generation;
DROP TABLE IF EXISTS fact_session;
DROP TABLE IF EXISTS fact_tool_call;
DROP TABLE IF EXISTS fact_turn;

DROP FUNCTION IF EXISTS reporting_capability_usage_key(UUID, JSONB, JSONB, JSONB, BIGINT);
