-- drop_turn_workflow_signals
--
-- Platform turns run only as session actors (knowledge/foundations/runtime.md).
-- The per-step turn workflow was the only writer and reader of durable
-- workflow signals (USER_MESSAGE steering) and the only caller of the
-- deduplicated `waiting_turn_resolution_*` enqueue this unique index backed.
-- Dropping the table also drops its sequence, index, primary key and foreign
-- key. Historic `turn_workflow` instance rows stay: they are ordinary
-- terminal workflows the admin views still list.
DROP TABLE IF EXISTS durable_signals;
DROP INDEX IF EXISTS idx_durable_task_queue_waiting_turn_resolution;
