-- Retire the `app` budget subject type (EVE-1129).
--
-- `app_channel` deliberately stays. See the note below the conversion.
--
-- Migration 137 (EVE-1004) fanned each `app` budget out to one `agent_endpoint`
-- budget per endpoint and deliberately left the original in place, still
-- enforced, so the aggregate ceiling kept binding across those endpoints until
-- the subject type could be dropped. This is that drop, and it has to carry the
-- ceiling somewhere or it is a spend increase.
--
-- `check_budgets_for_session` evaluates every budget the hierarchy matches and
-- keeps the most restrictive result, so an App with a $10 cap and three
-- endpoints is capped at $10 in aggregate today, on top of $10 per endpoint.
-- Deleting the `app` level with no successor would let that org spend $30
-- without changing anything on their side.
--
-- An App fronted exactly one Agent, and `agent` is a live subject type, so the
-- aggregate ceiling has an exact home.

-- ---------------------------------------------------------------------------
-- Convert `app` budgets onto the agent
-- ---------------------------------------------------------------------------

-- Inserted unconditionally rather than skipped when the agent already has a
-- budget. Budgets do not stack into a larger allowance: there is no unique
-- constraint on (org_id, subject_type, subject_id), and every matching row is
-- evaluated with the most restrictive winning, so two rows on one agent bind as
-- the tighter of the two. Skipping would therefore *loosen* the ceiling
-- whenever the agent's existing budget is the looser one, which is the outcome
-- this migration exists to prevent.
--
-- `period_started_at` is carried across unchanged, as 137 did: resetting an
-- in-flight window would hand back a fresh allowance. `balance` comes across
-- too, so spend already recorded against the App cap still counts against it.
--
-- The subject is the agent's `public_id`, which is what the hierarchy resolver
-- looks up (EVE-1136) and what the API accepts.
INSERT INTO budgets (
    org_id, subject_type, subject_id, currency, "limit", soft_limit,
    balance, period, metadata, status, period_started_at
)
SELECT
    b.org_id,
    'agent',
    a.public_id,
    b.currency,
    b."limit",
    b.soft_limit,
    b.balance,
    b.period,
    COALESCE(b.metadata, '{}'::jsonb) || jsonb_build_object(
        'converted_from', 'app',
        'converted_from_subject_id', b.subject_id
    ),
    b.status,
    b.period_started_at
FROM budgets AS b
JOIN apps AS app ON app.public_id = b.subject_id AND app.org_id = b.org_id
JOIN agents AS a ON a.id = app.agent_id AND a.org_id = b.org_id
WHERE b.subject_type = 'app';

-- An `app` budget with no reachable agent has no conversion target. That is
-- only sound because it also constrained nothing: `apps.agent_id` is nullable
-- for draft Apps (migration 019), and an App with no agent never produced a
-- session, so no spend could ever have been attributed to it. Recorded spend
-- would contradict that premise, so fail loudly rather than drop a cap that was
-- doing work.
DO $$
DECLARE
    spent_unconvertible BIGINT;
BEGIN
    SELECT COUNT(*)
    INTO spent_unconvertible
    FROM budgets AS b
    LEFT JOIN apps AS app
           ON app.public_id = b.subject_id AND app.org_id = b.org_id
    LEFT JOIN agents AS a
           ON a.id = app.agent_id AND a.org_id = b.org_id
    WHERE b.subject_type = 'app'
      AND a.public_id IS NULL
      AND b.balance < b."limit";

    IF spent_unconvertible > 0 THEN
        RAISE EXCEPTION
            'app budgets with recorded spend have no agent to convert onto: %',
            spent_unconvertible;
    END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- Why `app_channel` is not retired here
-- ---------------------------------------------------------------------------
--
-- 137 converted every `app_channel` budget to `agent_endpoint` 1:1, which would
-- suggest the level is dead. Migration 138 then undid exactly that: it moved
-- App webhooks off `agent_endpoints` and onto `agent_triggers`, moved their
-- budgets back from `agent_endpoint` to `app_channel`, and deleted the endpoint
-- rows those budgets had briefly been keyed on.
--
-- So for a webhook trigger, `app_channel` is not legacy tolerance — it is the
-- current and only attribution, and `trigger_session_tags` in
-- `domains/agent_triggers/commands.rs` still writes the `app_channel:` tag that
-- reaches it. There is no structural successor to convert onto, because 138
-- deleted the endpoint. Retiring the level here would delete live, enforced
-- webhook caps, which is the same harm this migration exists to prevent, one
-- level down. It needs a structural subject for trigger ingress first.

-- ---------------------------------------------------------------------------
-- Drop the retired level
-- ---------------------------------------------------------------------------

DELETE FROM budgets WHERE subject_type = 'app';

ALTER TABLE budgets
    DROP CONSTRAINT IF EXISTS budgets_subject_type_check;

ALTER TABLE budgets
    ADD CONSTRAINT budgets_subject_type_check
    CHECK (subject_type IN ('session', 'agent', 'user', 'org', 'app_channel', 'agent_endpoint'));
