-- Session-level client hints (EVE-161)
--
-- Generic key-value hints from the client. Nullable JSONB column; NULL means
-- no session-level hints (per-message controls.hints still apply).

ALTER TABLE sessions ADD COLUMN hints JSONB;
