-- Retire the `app` payment-policy subject (EVE-1130).
--
-- EVE-1004 moved budgets off App-shaped subjects and never reached payment
-- policies, which are a separate subsystem with their own subject-type list.
-- Until now the API, the CHECK constraint and the UI dropdown all still
-- accepted `app`, while `agent_endpoint` — the subject that replaced it
-- everywhere else — was not offered at all.
--
-- Worth being precise about what was broken: an `app` policy was accepted and
-- stored, but `subject_candidates` in `domains/payments/authority.rs` never
-- produced an `app` pair, and a policy authorizes a payment only on an exact
-- `(subject_type, subject_id)` match. So these policies have never authorized
-- anything. They are converted rather than deleted anyway, because the operator
-- who wrote one expressed an intent about spend and is entitled to have it
-- start working rather than vanish.

-- An App fronted exactly one Agent, so `agent` preserves the intent — the same
-- reasoning as the budget conversion in 151. The subject is the agent's
-- `public_id`, which is what `subject_candidates` matches on and what the API
-- accepts.
--
-- `NOT EXISTS` guards against creating a second policy where an equivalent one
-- already exists: unlike a budget, where every matching row binds and the
-- tightest wins, a payment policy is permissive — the first match that allows
-- the request authorizes it. Duplicating one cannot tighten anything, and would
-- leave the operator two rows to keep in sync.
INSERT INTO payment_policies (
    org_id, payment_account_id, subject_type, subject_id,
    allowed_capabilities, allowed_hosts, rail_preference,
    max_amount_usd_per_request, max_amount_usd_per_turn, max_amount_usd_per_day,
    require_approval_above_usd, status, metadata
)
SELECT
    p.org_id,
    p.payment_account_id,
    'agent',
    a.public_id,
    p.allowed_capabilities,
    p.allowed_hosts,
    p.rail_preference,
    p.max_amount_usd_per_request,
    p.max_amount_usd_per_turn,
    p.max_amount_usd_per_day,
    p.require_approval_above_usd,
    p.status,
    COALESCE(p.metadata, '{}'::jsonb) || jsonb_build_object(
        'converted_from', 'app',
        'converted_from_subject_id', p.subject_id
    )
FROM payment_policies AS p
JOIN apps AS app ON app.public_id = p.subject_id AND app.org_id = p.org_id
JOIN agents AS a ON a.id = app.agent_id AND a.org_id = p.org_id
WHERE p.subject_type = 'app'
  AND NOT EXISTS (
      SELECT 1
      FROM payment_policies AS existing
      WHERE existing.org_id = p.org_id
        AND existing.subject_type = 'agent'
        AND existing.subject_id = a.public_id
        AND existing.payment_account_id = p.payment_account_id
  );

-- An `app` policy with no reachable agent has no conversion target.
-- `apps.agent_id` is nullable for draft Apps (migration 019), and a draft App
-- never ran a session, so nothing could have spent under it. It authorized
-- nothing before this migration and authorizes nothing after, so it is dropped
-- with the rest below rather than raising: unlike a budget, there is no
-- recorded-spend signal on a policy that could contradict the premise.

DELETE FROM payment_policies WHERE subject_type = 'app';

ALTER TABLE payment_policies
    DROP CONSTRAINT IF EXISTS payment_policies_subject_type_check;

ALTER TABLE payment_policies
    ADD CONSTRAINT payment_policies_subject_type_check
    CHECK (subject_type IN ('user', 'agent_identity', 'agent', 'agent_endpoint', 'session', 'org'));
