-- Data plane v2, Phase C: running v1 sessions moved onto their org's data
-- plane (`domains/data_plane/moves.rs`). A row per session moved, kept as
-- its record: where its v1 log ends (the events stay in `events`), what
-- the move carried and what it left behind. `state` is the step a resumed
-- move continues from: `switched` (its messages already go to the data
-- plane, which refuses them until the import is done) or `done`.
CREATE TABLE dp_session_moves (
    session_id TEXT PRIMARY KEY,
    org_id BIGINT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('switched', 'done')),
    -- The last v1 event the move read; later events are the cell's.
    from_sequence INTEGER,
    -- What the move carried and left behind (`moves::MoveReport`).
    report JSONB,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ
);
CREATE INDEX dp_session_moves_org ON dp_session_moves (org_id, started_at);
