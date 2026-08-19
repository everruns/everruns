-- Let `DELETE /v1/sessions/{session_id}` succeed on PostgreSQL (EVE-919).
--
-- Deleting a session fans out through foreign keys into tables that carry
-- append-only guards, and every one of those guards aborted the delete:
--
--   * events            — FK ON DELETE CASCADE fires the BEFORE DELETE guard
--   * usage_journal     — FK ON DELETE SET NULL fires the BEFORE UPDATE guard
--   * usage_ledger      — same as usage_journal
--   * eval_case_results — FK declares no ON DELETE action, so it restricts
--
-- Any session that has taken a single turn has both events and usage rows, so
-- the DELETE always aborted and the API answered 500. The in-memory backend
-- cascades in Rust and has no triggers, which is why tests never caught it.
--
-- The guards themselves stay. What changes is that each one now recognises the
-- one legitimate way its rows are allowed to move: a session being purged.

-- ---------------------------------------------------------------------------
-- events: purging a session removes its history wholesale
-- ---------------------------------------------------------------------------
-- `app.archival_bypass` already exists for the retention archiver
-- (crates/server/src/event_retention.rs). Session deletion is the second
-- legitimate remover of events, and it is not archival, so it gets its own
-- flag rather than borrowing a name that would misdescribe it. Both bypasses
-- are scoped to `events`; budget_ledger shares this function and must keep
-- strict append-only semantics.
CREATE OR REPLACE FUNCTION prevent_event_mutation()
RETURNS TRIGGER AS $$
BEGIN
    IF TG_OP = 'DELETE'
       AND TG_TABLE_NAME = 'events'
       AND (current_setting('app.archival_bypass', true) = 'true'
            OR current_setting('app.session_purge', true) = 'true') THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'events are append-only: % operations are not allowed', TG_OP;
END;
$$ LANGUAGE plpgsql;

COMMENT ON FUNCTION prevent_event_mutation() IS
    'Enforces append-only semantics. On events, DELETE is permitted under app.archival_bypass (retention archival) or app.session_purge (session deletion); every other table using this function is strictly append-only.';

-- ---------------------------------------------------------------------------
-- usage_journal / usage_ledger: the financial record outlives the session
-- ---------------------------------------------------------------------------
-- Both already declare their references as ON DELETE SET NULL, which is
-- exactly the intended behaviour: keep the row, forget what produced it. The
-- shared events guard rejected those cascades because they arrive as UPDATEs.
-- This guard permits precisely that detachment — a reference column going from
-- a value to NULL — and rejects any other write, including no-op updates.
CREATE OR REPLACE FUNCTION prevent_usage_record_mutation()
RETURNS TRIGGER AS $$
DECLARE
    reference_columns CONSTANT text[] := ARRAY[
        'budget_id', 'event_id', 'session_id',
        'user_id', 'principal_id', 'agent_id', 'harness_id'
    ];
    detached integer;
    rewritten integer;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        SELECT
            count(*) FILTER (
                WHERE new_field.value = 'null'::jsonb
                  AND old_field.key = ANY (reference_columns)
            ),
            count(*) FILTER (
                WHERE new_field.value <> 'null'::jsonb
                   OR NOT old_field.key = ANY (reference_columns)
            )
        INTO detached, rewritten
        FROM jsonb_each(to_jsonb(OLD)) AS old_field
        JOIN jsonb_each(to_jsonb(NEW)) AS new_field ON new_field.key = old_field.key
        WHERE old_field.value IS DISTINCT FROM new_field.value;

        IF detached > 0 AND rewritten = 0 THEN
            RETURN NEW;
        END IF;
    END IF;

    RAISE EXCEPTION 'usage records are append-only: % operations are not allowed', TG_OP;
END;
$$ LANGUAGE plpgsql;

COMMENT ON FUNCTION prevent_usage_record_mutation() IS
    'Enforces append-only semantics on usage_journal and usage_ledger, except for ON DELETE SET NULL cascades that detach a deleted session, event, or other referenced row.';

DROP TRIGGER prevent_usage_journal_mutation ON usage_journal;
CREATE TRIGGER prevent_usage_journal_mutation
    BEFORE UPDATE OR DELETE ON usage_journal
    FOR EACH ROW
    EXECUTE FUNCTION prevent_usage_record_mutation();

DROP TRIGGER prevent_usage_ledger_mutation ON usage_ledger;
CREATE TRIGGER prevent_usage_ledger_mutation
    BEFORE UPDATE OR DELETE ON usage_ledger
    FOR EACH ROW
    EXECUTE FUNCTION prevent_usage_record_mutation();

-- ---------------------------------------------------------------------------
-- eval_case_results: eval history outlives the session it ran in
-- ---------------------------------------------------------------------------
-- The column is already nullable; the FK simply never declared an ON DELETE
-- action, so it restricted instead. Detaching matches the usage tables.
ALTER TABLE eval_case_results
    DROP CONSTRAINT eval_case_results_session_id_fk;

ALTER TABLE eval_case_results
    ADD CONSTRAINT eval_case_results_session_id_fk
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE SET NULL;
