-- Org-attributed inference with no session.
--
-- `llm_generations` is the single source of truth for inference spend, but it
-- assumed every call belongs to a conversation. Knowledge-index sync embeds
-- every chunk of every indexed document as host-triggered background work with
-- no session, so the larger half of embedding spend had nowhere to land and was
-- only logged (EVE-894, EVE-898).
--
-- Making `session_id` nullable keeps one ledger rather than a parallel
-- org-scoped table that every usage query would have to union in. Rows without
-- a session are org-attributed: `org_id` stays NOT NULL, so org usage reporting
-- and cost totals see them.
--
-- Session-scoped consumers are unaffected: the denormalized `sessions` /
-- `agents` totals and the session budget debit are keyed by session and simply
-- have no subject for these rows. Host-triggered background indexing does not
-- consume a cap the user did not spend against.
--
-- Additive only: every existing row has a session and keeps it.

ALTER TABLE llm_generations ALTER COLUMN session_id DROP NOT NULL;

COMMENT ON COLUMN llm_generations.session_id IS
    'Owning conversation, or NULL for org-attributed background inference (e.g. knowledge-index sync embeddings) that no session triggered.';

-- Org usage reporting reads these rows by org and time; `idx_llm_generations_org_created`
-- already serves that. This partial index serves the narrower "what background
-- spend did this org incur" question without scanning session-bound rows.
CREATE INDEX idx_llm_generations_org_sessionless
    ON llm_generations (org_id, created_at)
    WHERE session_id IS NULL;
