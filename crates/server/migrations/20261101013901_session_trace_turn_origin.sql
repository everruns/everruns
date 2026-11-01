-- Who wrote a trace turn's input when no person did (schedule run, trigger,
-- thread wake-up, platform notice), from `turn.started`'s `origin`. NULL means
-- a person started the turn, or the turn predates origins; no backfill.
ALTER TABLE session_trace_turns ADD COLUMN origin JSONB;
