-- One sentence saying what a run did, stored on the session (EVE-867).
--
-- The session detail header renders `Started 8/9/2026, 9:00:04 AM` under the
-- title. That is metadata; the IA concept calls for an explanation — on a failed
-- run, the most valuable text on the page. Deriving it per page load would mean
-- a paid LLM call on every read, and three surfaces (header, the sessions list
-- failure column, failure notifications) each inventing their own wording, so it
-- is generated once and stored.
--
-- Two additive columns:
--
--   `run_summary`                 — the generated sentence. NULL is the normal
--                                   resting state, not an error: deployments
--                                   without a utility LLM never write it, and
--                                   the header falls back to the meta line.
--   `run_summary_turn_sequence`   — the `events.sequence` of the terminal turn
--                                   the summary describes.
--
-- The sequence column is the fence. Summarisation runs out of band after the
-- terminal turn, so a slow call for turn N can land after turn N+1 has already
-- been summarised. Writes compare against this column and refuse to move it
-- backwards, the same guard `sessions_last_turn_insert` applies to
-- `last_turn_sequence` in 118_session_source_and_last_turn.sql.
--
-- Deliberately no trigger. Unlike `last_turn_*`, this value cannot be derived
-- from the events table by SQL; it is written by the application after an LLM
-- call that may never happen.

ALTER TABLE sessions
    ADD COLUMN run_summary TEXT,
    ADD COLUMN run_summary_turn_sequence BIGINT;

COMMENT ON COLUMN sessions.run_summary IS
    'Generated one-sentence description of what the run did, and where it '
    'failed (EVE-867). NULL when no utility LLM is configured, when the '
    'session is a chat thread, or before the first terminal turn.';

COMMENT ON COLUMN sessions.run_summary_turn_sequence IS
    'events.sequence of the terminal turn `run_summary` describes. Summaries '
    'are generated out of band, so this fences a late write for an older turn '
    'from overwriting a newer summary.';
