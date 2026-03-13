-- Everruns: session locale support
--
-- Adds an optional locale tag to sessions so agent execution can carry
-- language and regional formatting preferences across API, worker, and
-- scheduled/subagent turns.

ALTER TABLE sessions ADD COLUMN locale TEXT;

COMMENT ON COLUMN sessions.locale IS 'Optional BCP 47 locale tag for localized agent behavior (for example uk-UA)';
