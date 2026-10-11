-- Session schedule claim lease: make the poller safe to run on more than one
-- server instance.
--
-- `claim_due_session_schedules` selected due rows with `FOR UPDATE SKIP LOCKED`
-- but ran the statement straight on the pool, so its implicit transaction
-- committed and released the row locks before the caller had advanced
-- `next_trigger_at`. Nothing recorded that a row had been picked up, so a
-- second instance polling in that window saw the same schedules as still due
-- and fired them again: duplicate agent turns and duplicate monitor probes.
--
-- These columns let the claim be a single atomic statement that both selects
-- and stamps the row, the same shape `durable_schedules` already uses. A claim
-- is a lease rather than a permanent mark, so an instance that dies mid-fire
-- releases its schedules once the lease ages out instead of stranding them.
--
-- No index is added: idx_session_schedules_polling already covers the
-- (enabled, next_trigger_at) predicate this claim filters on.
ALTER TABLE session_schedules
    ADD COLUMN claimed_by TEXT,
    ADD COLUMN claimed_at TIMESTAMPTZ;
