-- Per-tool-call idempotency store for the Act activity (EVE-530).
--
-- Each row represents one tool call dispatch attempt within a durable turn.
-- The UNIQUE constraint on (turn_id, tool_call_id) is the CAS primitive:
-- the first INSERT wins (claim), subsequent ones conflict and reveal the
-- existing status so callers can decide to replay or skip.
--
-- status values:
--   'running'     – claimed but not yet settled
--   'settled'     – completed; result_json holds the tool result
--   'interrupted' – AtMostOnce tool whose running claim became stale; not re-run
CREATE TABLE durable_tool_results (
    id             UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    turn_id        TEXT         NOT NULL,
    tool_call_id   TEXT         NOT NULL,
    tool_name      TEXT         NOT NULL,
    args_fingerprint TEXT       NOT NULL,
    status         TEXT         NOT NULL DEFAULT 'running'
                   CHECK (status IN ('running', 'settled', 'interrupted')),
    -- Opaque token generated at claim time; settle CAS verifies this matches.
    claim_token    UUID         NOT NULL,
    result_json    JSONB,
    created_at     TIMESTAMPTZ  NOT NULL DEFAULT now(),
    settled_at     TIMESTAMPTZ,
    CONSTRAINT durable_tool_results_turn_call_uq UNIQUE (turn_id, tool_call_id)
);

COMMENT ON TABLE durable_tool_results IS
    'Per-tool-call idempotency records for the Act durable activity (EVE-530). '
    'Prevents double-execution of AtMostOnce tools on worker reclaim/replay.';

CREATE INDEX durable_tool_results_turn_idx
    ON durable_tool_results (turn_id);
