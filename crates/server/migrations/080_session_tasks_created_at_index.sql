-- Org-scoped task listing (EVE-583).
--
-- `list_org_tasks` lists every task across an org's sessions, newest-first,
-- with optional kind/state/age filters and a bounded LIMIT (ops/observability
-- dashboards). The org boundary is a semijoin on `sessions(org_id)`; this index
-- lets the recency ordering and the `created_at >= cutoff` age bound be served
-- without a full-table sort as `session_tasks` grows. The existing
-- (session_id, state) index only helps the per-session path.
CREATE INDEX session_tasks_created_at_idx ON session_tasks (created_at DESC);
