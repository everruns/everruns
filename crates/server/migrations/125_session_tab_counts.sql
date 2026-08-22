-- Denormalized counts behind the session detail tab badges (EVE-868).
--
-- The tab bar renders on every session page load, so a COUNT(*) per tab over
-- `events` is not acceptable — bounded event reads were their own piece of work
-- (EVE-730) and the sessions list has been through two rounds of query surgery
-- (EVE-695, EVE-697). Migration 102 already established the shape for this:
-- incremental counters on `sessions` maintained by statement-level triggers, so
-- the read side is O(1) fields on a row the page already fetches.
--
-- Three counters, each on the table that owns the thing it counts:
--
--   * sessions.event_count  — every event in the session (Events tab).
--   * sessions.task_count   — background work the session owns (Work tab):
--                             subagents, external agents, background tools.
--   * workspaces.file_count — non-directory files (Workspace tab). This lives
--                             on `workspaces`, not `sessions`, because files
--                             were rekeyed to the workspace in migration 056
--                             and a workspace can back more than one session
--                             (multi-head workspaces, EVE-907). A per-session
--                             counter would double-count or drift there.
--
-- Directories are excluded from file_count: the badge answers "is there
-- anything to look at", and an empty tree of directories is not.

ALTER TABLE sessions
    ADD COLUMN event_count BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN task_count BIGINT NOT NULL DEFAULT 0;

COMMENT ON COLUMN sessions.event_count IS
    'Denormalized: total events recorded for this session (EVE-868 tab badge)';
COMMENT ON COLUMN sessions.task_count IS
    'Denormalized: total session_tasks owned by this session (EVE-868 tab badge)';

ALTER TABLE workspaces
    ADD COLUMN file_count BIGINT NOT NULL DEFAULT 0;

COMMENT ON COLUMN workspaces.file_count IS
    'Denormalized: non-directory workspace_files rows (EVE-868 tab badge)';

-- ---------------------------------------------------------------------------
-- One-time backfill from the canonical tables, before triggers take over.
-- ---------------------------------------------------------------------------

UPDATE sessions s
SET event_count = counts.event_count
FROM (
    SELECT session_id, count(*) AS event_count
    FROM events
    GROUP BY session_id
) counts
WHERE s.id = counts.session_id;

UPDATE sessions s
SET task_count = counts.task_count
FROM (
    SELECT session_id, count(*) AS task_count
    FROM session_tasks
    GROUP BY session_id
) counts
WHERE s.id = counts.session_id;

UPDATE workspaces w
SET file_count = counts.file_count
FROM (
    SELECT workspace_id, count(*) AS file_count
    FROM workspace_files
    WHERE NOT is_directory
    GROUP BY workspace_id
) counts
WHERE w.id = counts.workspace_id;

-- ---------------------------------------------------------------------------
-- events → sessions.event_count
-- ---------------------------------------------------------------------------
-- Statement-level with transition tables, matching migration 102: one UPDATE
-- per statement rather than per row, so a batch insert of a long turn's events
-- costs one extra update.

CREATE OR REPLACE FUNCTION sessions_event_count_insert()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE sessions s
    SET event_count = s.event_count + delta.event_count
    FROM (
        SELECT session_id, count(*) AS event_count
        FROM new_rows
        GROUP BY session_id
    ) delta
    WHERE s.id = delta.session_id;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION sessions_event_count_delete()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE sessions s
    SET event_count = GREATEST(s.event_count - delta.event_count, 0)
    FROM (
        SELECT session_id, count(*) AS event_count
        FROM old_rows
        GROUP BY session_id
    ) delta
    WHERE s.id = delta.session_id;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sessions_event_count_insert
    AFTER INSERT ON events
    REFERENCING NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION sessions_event_count_insert();

CREATE TRIGGER sessions_event_count_delete
    AFTER DELETE ON events
    REFERENCING OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION sessions_event_count_delete();

-- ---------------------------------------------------------------------------
-- session_tasks → sessions.task_count
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION sessions_task_count_insert()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE sessions s
    SET task_count = s.task_count + delta.task_count
    FROM (
        SELECT session_id, count(*) AS task_count
        FROM new_rows
        GROUP BY session_id
    ) delta
    WHERE s.id = delta.session_id;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION sessions_task_count_delete()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE sessions s
    SET task_count = GREATEST(s.task_count - delta.task_count, 0)
    FROM (
        SELECT session_id, count(*) AS task_count
        FROM old_rows
        GROUP BY session_id
    ) delta
    WHERE s.id = delta.session_id;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER session_tasks_count_insert
    AFTER INSERT ON session_tasks
    REFERENCING NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION sessions_task_count_insert();

CREATE TRIGGER session_tasks_count_delete
    AFTER DELETE ON session_tasks
    REFERENCING OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION sessions_task_count_delete();

-- ---------------------------------------------------------------------------
-- workspace_files → workspaces.file_count
-- ---------------------------------------------------------------------------
-- `is_directory` is settable on insert and a row can in principle be updated
-- from one to the other, so the UPDATE trigger keeps the counter honest rather
-- than assuming the flag is immutable.

CREATE OR REPLACE FUNCTION workspaces_file_count_insert()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE workspaces w
    SET file_count = w.file_count + delta.file_count
    FROM (
        SELECT workspace_id, count(*) AS file_count
        FROM new_rows
        WHERE NOT is_directory
        GROUP BY workspace_id
    ) delta
    WHERE w.id = delta.workspace_id;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION workspaces_file_count_delete()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE workspaces w
    SET file_count = GREATEST(w.file_count - delta.file_count, 0)
    FROM (
        SELECT workspace_id, count(*) AS file_count
        FROM old_rows
        WHERE NOT is_directory
        GROUP BY workspace_id
    ) delta
    WHERE w.id = delta.workspace_id;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION workspaces_file_count_update()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE workspaces w
    SET file_count = GREATEST(w.file_count + delta.net, 0)
    FROM (
        SELECT
            workspace_id,
            sum(net) AS net
        FROM (
            SELECT workspace_id, count(*) AS net
            FROM new_rows
            WHERE NOT is_directory
            GROUP BY workspace_id
            UNION ALL
            SELECT workspace_id, -count(*) AS net
            FROM old_rows
            WHERE NOT is_directory
            GROUP BY workspace_id
        ) per_side
        GROUP BY workspace_id
    ) delta
    WHERE w.id = delta.workspace_id
      AND delta.net <> 0;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER workspace_files_count_insert
    AFTER INSERT ON workspace_files
    REFERENCING NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION workspaces_file_count_insert();

CREATE TRIGGER workspace_files_count_delete
    AFTER DELETE ON workspace_files
    REFERENCING OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION workspaces_file_count_delete();

CREATE TRIGGER workspace_files_count_update
    AFTER UPDATE ON workspace_files
    REFERENCING NEW TABLE AS new_rows OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION workspaces_file_count_update();
