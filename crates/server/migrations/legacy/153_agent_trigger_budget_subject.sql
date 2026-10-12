-- EVE-1138: give trigger ingress a structural budget subject and retire
-- `app_channel`, the last budget subject resolved from a session tag.
--
-- Migration 137 converted every `app_channel` budget to `agent_endpoint` and
-- asserted none remained. Migration 138 then moved App webhooks onto
-- `agent_triggers` and moved exactly those budgets back to `app_channel`
-- before deleting the endpoint rows they had been keyed on. So for a webhook
-- trigger `app_channel` is the current and only attribution, with no
-- structural successor to convert onto — until this migration adds one.
--
-- Subject identity: the trigger's API-facing id (`trg_<hex>`, rendered from
-- `agent_triggers.id`), not `agent_triggers.ingress_id`. EVE-1138 proposed
-- `ingress_id` because it is unique and is what the surviving budgets are
-- already keyed by, which would make this a pure rename. It is the wrong key
-- for two reasons. `ingress_id` is a nullable compatibility column carried
-- only by the webhooks migration 138 moved, so a trigger created today has
-- none and could never be given a budget. And EVE-1136 established the rule
-- the hard way: a budget subject must be keyed by the identifier the API
-- exposes, because that is the only identifier an operator can put in a
-- budget. For a trigger that is `trg_<hex>`. The conversion below therefore
-- re-keys rather than renames.

-- ---------------------------------------------------------------------------
-- Sessions: structural trigger attribution
-- ---------------------------------------------------------------------------

ALTER TABLE sessions
    ADD COLUMN trigger_id UUID REFERENCES agent_triggers(id) ON DELETE SET NULL;

COMMENT ON COLUMN sessions.trigger_id IS
    'Agent trigger whose ingress created this session (EVE-1138). NULL for '
    'user, API, endpoint and platform-created sessions. Replaces the '
    'app_channel: session tag as the budget attribution for trigger ingress.';

-- Backfill from the two server-written tags that already name a trigger.
--
-- Non-webhook trigger sessions carry `agent_trigger:trg_<hex>` directly.
-- `TriggerId` renders as the prefix plus the UUID with its dashes removed, so
-- the tag is reversed by stripping the prefix and re-inserting the dashes.
UPDATE sessions AS s
SET trigger_id = t.id
FROM agent_triggers AS t
WHERE s.trigger_id IS NULL
  AND s.org_id = t.org_id
  AND ('agent_trigger:trg_' || REPLACE(t.id::text, '-', '')) = ANY (s.tags);

-- Webhook trigger sessions carry `app_channel:<ingress_id>`, written by
-- migration 138 and still written today by `trigger_session_tags`.
UPDATE sessions AS s
SET trigger_id = t.id
FROM agent_triggers AS t
WHERE s.trigger_id IS NULL
  AND s.org_id = t.org_id
  AND t.ingress_id IS NOT NULL
  AND ('app_channel:' || t.ingress_id) = ANY (s.tags);

CREATE INDEX idx_sessions_trigger_id ON sessions(trigger_id)
    WHERE trigger_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Budgets: add the subject, convert onto it, drop `app_channel`
-- ---------------------------------------------------------------------------

ALTER TABLE budgets
    DROP CONSTRAINT IF EXISTS budgets_subject_type_check;

-- Convert in place: same row, same limit, soft limit, balance, currency,
-- period and `period_started_at`, so an in-flight window is not reset to now
-- and does not hand back a fresh allowance. Only the subject type and the
-- identifier move. The original subject id is kept in metadata so an operator
-- can see where the row came from.
UPDATE budgets AS b
SET subject_type = 'agent_trigger',
    subject_id = 'trg_' || REPLACE(t.id::text, '-', ''),
    metadata = COALESCE(b.metadata, '{}'::jsonb) || jsonb_build_object(
        'converted_from', 'app_channel',
        'converted_from_subject_id', b.subject_id
    ),
    updated_at = NOW()
FROM agent_triggers AS t
WHERE b.subject_type = 'app_channel'
  AND t.org_id = b.org_id
  AND t.ingress_id = b.subject_id;

-- An `app_channel` budget with no matching trigger is an enforced ceiling we
-- cannot re-key. Deleting it would loosen that org's effective ceiling, which
-- is the exact harm EVE-1129 and this issue exist to prevent, so fail the
-- migration loudly instead and let a human decide. Nothing can have created
-- one since 138: `validate_subject_type` has never accepted `app_channel` on
-- the write path.
DO $$
DECLARE
    unconverted BIGINT;
BEGIN
    SELECT COUNT(*)
    INTO unconverted
    FROM budgets
    WHERE subject_type = 'app_channel';

    IF unconverted > 0 THEN
        RAISE EXCEPTION
            'app_channel budgets remain with no matching agent_trigger.ingress_id: %',
            unconverted;
    END IF;
END;
$$;

ALTER TABLE budgets
    ADD CONSTRAINT budgets_subject_type_check
    CHECK (subject_type IN ('session', 'agent', 'user', 'org', 'agent_trigger', 'agent_endpoint'));
