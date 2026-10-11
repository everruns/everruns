-- Runnable index for sessions as actors (actor-based design, step 6b).
--
-- Decisions:
-- - One row per session that has ever had input for a worker. A session is an
--   actor; its event log is its only turn state, so this table holds no turn
--   state at all: only whether the actor has work (`runnable_since`), which
--   input it must see (`inbox_seq`), who runs it (the lease), and a pending
--   cancel. Resume is a fold of the log, never a read of this row.
-- - `runnable_since` NULL means nothing to do. It is set by the transaction
--   that persists an input and cleared only by a release whose holder consumed
--   every input up to `inbox_seq`, so an input that lands while a turn is
--   finishing keeps the actor runnable instead of being lost.
-- - `inbox_seq` is the event sequence of the latest input the actor must see;
--   it only moves forward.
-- - The lease is a holder, a fence and an expiry. Every lease raises the fence,
--   so a holder that stalled past its expiry and lost the actor cannot renew or
--   release the new holder's lease. Expiry is database time (`now()`), so
--   worker clocks never matter.
-- - `cancel_reported` records that a lease or renew told the holder about the
--   pending cancel. A release clears the cancel only once the holder was told,
--   so a cancel that lands between the last renew and the release survives
--   and is reported to the next holder.
-- - `org_id` is denormalized from `sessions` so the lease query can cap the
--   number of actors one org holds at once without a join.
-- - The lease index is partial on runnable rows: the lease query scans only
--   actors with work, in arrival order, and filters free or expired leases
--   from that small set (an expiry predicate cannot sit in an index predicate,
--   since it depends on `now()`).
-- Spec: knowledge/operations/durable-execution-engine.md ("Session actors (runnable index)").

CREATE TABLE session_actors (
    session_id UUID PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    runnable_since TIMESTAMPTZ,
    inbox_seq BIGINT NOT NULL DEFAULT 0,
    lease_holder TEXT,
    lease_fence BIGINT NOT NULL DEFAULT 0,
    lease_expires_at TIMESTAMPTZ,
    cancel_requested_at TIMESTAMPTZ,
    cancel_reported BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK ((lease_holder IS NULL) = (lease_expires_at IS NULL))
);

-- The lease query: runnable actors, oldest first.
CREATE INDEX idx_session_actors_runnable
    ON session_actors (runnable_since, session_id)
    WHERE runnable_since IS NOT NULL;

-- Per-org count of held leases for the fairness cap.
CREATE INDEX idx_session_actors_leased_org
    ON session_actors (org_id)
    WHERE lease_holder IS NOT NULL;

CREATE TRIGGER update_session_actors_updated_at BEFORE UPDATE ON session_actors
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
