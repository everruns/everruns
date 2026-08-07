-- Per-task push-notification configs (EVE-682).
--
-- Unlike organization_task_webhooks (org-scoped, terminal-only), these targets
-- are scoped to a single session task and can opt into non-terminal events via
-- `event_filter`. Modeled on A2A TaskPushNotificationConfig.
--
-- Authorization has no org_id column: a config is reachable only through its
-- owning session, so callers are authorized via the session's org (the same
-- boundary the notifier already resolves). The FK to session_tasks(id) ON
-- DELETE CASCADE reuses the task lifecycle for cleanup (prune drops configs
-- with their task). See specs/session-tasks.md.

CREATE TABLE session_task_push_configs (
    id           BIGSERIAL PRIMARY KEY,
    public_id    TEXT NOT NULL UNIQUE,
    session_id   UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    task_id      TEXT NOT NULL REFERENCES session_tasks(id) ON DELETE CASCADE,
    url          TEXT NOT NULL,
    secret       TEXT,
    -- Events that trigger delivery. Defaults to terminal-only, matching org
    -- webhook behavior. Valid members: 'terminal', 'awaiting_input', 'message'.
    event_filter TEXT[] NOT NULL DEFAULT '{terminal}',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX session_task_push_configs_session_task_idx
    ON session_task_push_configs (session_id, task_id);
