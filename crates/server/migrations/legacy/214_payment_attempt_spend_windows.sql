-- Payment policy per-turn and per-day limits (machine payments).
--
-- Decision: limits are enforced against `payment_attempts` itself rather than a
-- separate budget ledger. Each attempt now records the policy that authorized it
-- and the input message (turn) it belongs to. The authority reserves a `pending`
-- attempt under a row lock on the policy before any wallet signs, so concurrent
-- tool calls in the same turn cannot both pass the check.
ALTER TABLE payment_attempts
    ADD COLUMN payment_policy_id UUID REFERENCES payment_policies(id) ON DELETE SET NULL,
    ADD COLUMN input_message_id UUID;

CREATE INDEX idx_payment_attempts_policy_created
    ON payment_attempts(payment_policy_id, created_at DESC)
    WHERE payment_policy_id IS NOT NULL;
