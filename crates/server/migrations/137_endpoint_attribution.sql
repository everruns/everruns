-- Phase 4b of retiring the App abstraction (EVE-1004): sessions and budgets
-- follow the exposure down to the endpoint.
--
-- `sessions.app_id` and the App-shaped budget subjects both record "which
-- bundle", when the useful grain is "which exposure". One App with a Slack
-- channel and a public chat channel produces two very different session
-- populations that are indistinguishable today, and a cap on the App says
-- nothing about which door the spend came through.
--
-- `sessions.app_id` and the `app`/`app_channel` subject types are deliberately
-- left in place; the deletion phase (EVE-1011) removes them.

-- ---------------------------------------------------------------------------
-- Sessions
-- ---------------------------------------------------------------------------

ALTER TABLE sessions
    ADD COLUMN endpoint_id UUID REFERENCES agent_endpoints(id) ON DELETE SET NULL;

COMMENT ON COLUMN sessions.endpoint_id IS
    'Endpoint whose ingress created this session. NULL for user, API, and '
    'platform-created sessions, and for app-channel sessions predating the '
    'routing tag that identifies the channel (EVE-1004).';

-- Backfill from the routing tag, not from `app_id`.
--
-- `app_id` alone is ambiguous: an App may own several endpoints, so it cannot
-- say which door a session came through. The endpoint routing tag can, and it
-- is the same value the reuse lookup already keys on. Sessions without such a
-- tag stay NULL rather than being guessed into an endpoint — the rule in
-- knowledge/runtime-resources/session-source-and-facets.md is to derive
-- structurally or leave unknown, never to infer.
--
-- Three spellings, because the tag convention grew per transport rather than
-- centrally: the app-invocation channels (webhook, schedule, A2A, api_endpoint)
-- write `app_channel:<public_id>`, Slack writes `slack:endpoint:<public_id>`,
-- and FCP writes `fcp:endpoint:<public_id>`. All three are server-written and
-- carry the same endpoint public id, so all three are structural evidence.
UPDATE sessions AS s
SET endpoint_id = ae.id
FROM agent_endpoints AS ae
WHERE s.endpoint_id IS NULL
  AND s.app_id = ae.app_id
  AND (
        ('app_channel:' || ae.public_id) = ANY (s.tags)
     OR ('slack:endpoint:' || ae.public_id) = ANY (s.tags)
     OR ('fcp:endpoint:' || ae.public_id) = ANY (s.tags)
  );

-- An App with exactly one endpoint is unambiguous even without the tag: there
-- is only one door the session could have come through. This is a derivation,
-- not a guess, and it is what recovers sessions created before the routing tag
-- existed.
UPDATE sessions AS s
SET endpoint_id = single.id
FROM (
    -- No MIN() over uuid in PostgreSQL, and the HAVING guarantees one row
    -- per group anyway, so take the only element.
    SELECT app_id, (ARRAY_AGG(id))[1] AS id
    FROM agent_endpoints
    GROUP BY app_id
    HAVING COUNT(*) = 1
) AS single
WHERE s.endpoint_id IS NULL
  AND s.app_id = single.app_id;

CREATE INDEX idx_sessions_endpoint_id ON sessions(endpoint_id)
    WHERE endpoint_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Budgets
-- ---------------------------------------------------------------------------

ALTER TABLE budgets
    DROP CONSTRAINT IF EXISTS budgets_subject_type_check;

ALTER TABLE budgets
    ADD CONSTRAINT budgets_subject_type_check
    CHECK (subject_type IN ('session', 'agent', 'user', 'org', 'app', 'app_channel', 'agent_endpoint'));

-- `app_channel` budgets convert 1:1. `subject_id` is the `appchan_` public id,
-- which the endpoint carried over unchanged in 135, so the identifier does not
-- move — only the subject type's name does.
UPDATE budgets
SET subject_type = 'agent_endpoint',
    updated_at = NOW()
WHERE subject_type = 'app_channel';

-- `app` budgets have no 1:1 successor, because the bundle they scoped is going
-- away. Fan each one out to a budget per endpoint of that App, preserving the
-- limit, the balance, and — critically — `period_started_at`, so an in-flight
-- window is not silently reset to now and does not hand back a fresh
-- allowance.
--
-- The cap per endpoint is preserved rather than divided. Dividing would
-- tighten every existing cap without consent; preserving means an App with N
-- endpoints can now spend up to N times its old ceiling in the worst case.
-- That is the direction an operator can observe and correct, and it is the
-- same shape the per-endpoint model has going forward. The App budget is kept
-- alongside (still enforced until EVE-1011 drops the subject type), so the
-- original ceiling continues to bind in the meantime.
INSERT INTO budgets (
    org_id, subject_type, subject_id, currency, "limit", soft_limit,
    balance, period, metadata, status, period_started_at
)
SELECT
    b.org_id,
    'agent_endpoint',
    ae.public_id,
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
JOIN agent_endpoints AS ae ON ae.app_id = app.id
WHERE b.subject_type = 'app'
  AND NOT EXISTS (
      SELECT 1
      FROM budgets AS existing
      WHERE existing.org_id = b.org_id
        AND existing.subject_type = 'agent_endpoint'
        AND existing.subject_id = ae.public_id
  );

DO $$
DECLARE
    unconverted BIGINT;
BEGIN
    SELECT COUNT(*)
    INTO unconverted
    FROM budgets
    WHERE subject_type = 'app_channel';

    IF unconverted > 0 THEN
        RAISE EXCEPTION 'app_channel budgets remain after conversion: %', unconverted;
    END IF;
END;
$$;
