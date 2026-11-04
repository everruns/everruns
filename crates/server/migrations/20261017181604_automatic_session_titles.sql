-- automatic_session_titles
ALTER TABLE sessions
    ADD COLUMN title_is_automatic BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN auto_title_sequence BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN auto_title_attempt_sequence BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN auto_title_input_sequence BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN auto_title_checked_at TIMESTAMPTZ,
    ADD COLUMN auto_title_backoff SMALLINT NOT NULL DEFAULT 0;
