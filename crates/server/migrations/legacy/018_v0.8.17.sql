-- principal_ownership: principal-based ownership for durable entities

----------------------------------------------------------------------
-- Principals
----------------------------------------------------------------------

CREATE TABLE principals (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    public_id TEXT NOT NULL UNIQUE,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('user', 'agent_identity', 'system')),
    subject_id UUID,
    parent_principal_id UUID REFERENCES principals(id) ON DELETE RESTRICT,
    resolved_user_id UUID REFERENCES users(id) ON DELETE RESTRICT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    CONSTRAINT principals_public_id_format CHECK (public_id ~ '^principal_[0-9a-f]{32}$')
);

CREATE UNIQUE INDEX idx_principals_org_kind_subject
    ON principals(org_id, kind, subject_id);
CREATE INDEX idx_principals_parent ON principals(parent_principal_id);
CREATE INDEX idx_principals_resolved_user ON principals(org_id, resolved_user_id);
CREATE INDEX idx_principals_kind_status ON principals(org_id, kind, status);

CREATE TRIGGER update_principals_updated_at
    BEFORE UPDATE ON principals
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

----------------------------------------------------------------------
-- Owner columns on first-wave durable entities
----------------------------------------------------------------------

ALTER TABLE sessions
    ADD COLUMN owner_principal_id UUID REFERENCES principals(id),
    ADD COLUMN resolved_owner_user_id UUID REFERENCES users(id);

ALTER TABLE session_schedules
    ADD COLUMN owner_principal_id UUID REFERENCES principals(id),
    ADD COLUMN resolved_owner_user_id UUID REFERENCES users(id);

ALTER TABLE apps
    ADD COLUMN owner_principal_id UUID REFERENCES principals(id),
    ADD COLUMN resolved_owner_user_id UUID REFERENCES users(id);

CREATE INDEX idx_sessions_owner_principal ON sessions(owner_principal_id);
CREATE INDEX idx_sessions_resolved_owner ON sessions(org_id, resolved_owner_user_id);
CREATE INDEX idx_session_schedules_owner_principal ON session_schedules(owner_principal_id);
CREATE INDEX idx_session_schedules_resolved_owner ON session_schedules(org_id, resolved_owner_user_id);
CREATE INDEX idx_apps_owner_principal ON apps(owner_principal_id);
CREATE INDEX idx_apps_resolved_owner ON apps(org_id, resolved_owner_user_id);

----------------------------------------------------------------------
-- Backfill principals
----------------------------------------------------------------------

WITH org_users AS (
    SELECT DISTINCT org_id, user_id
    FROM organization_members
    UNION
    SELECT org_id, created_by AS user_id
    FROM organizations
    WHERE created_by IS NOT NULL
),
inserted AS (
    INSERT INTO principals (
        id,
        public_id,
        org_id,
        kind,
        subject_id,
        resolved_user_id,
        metadata
    )
    SELECT
        uuidv7(),
        'principal_' || REPLACE(uuidv7()::text, '-', ''),
        ou.org_id,
        'user',
        ou.user_id,
        ou.user_id,
        jsonb_build_object('source', 'user_backfill')
    FROM org_users ou
    ON CONFLICT (org_id, kind, subject_id) DO NOTHING
    RETURNING 1
)
SELECT COUNT(*) FROM inserted;

WITH org_fallback_users AS (
    SELECT
        o.org_id,
        COALESCE(
            o.created_by,
            (
                SELECT om.user_id
                FROM organization_members om
                WHERE om.org_id = o.org_id AND om.role = 'owner'
                ORDER BY om.created_at ASC
                LIMIT 1
            ),
            (
                SELECT om.user_id
                FROM organization_members om
                WHERE om.org_id = o.org_id
                ORDER BY om.created_at ASC
                LIMIT 1
            )
        ) AS fallback_user_id
    FROM organizations o
),
inserted AS (
    INSERT INTO principals (
        id,
        public_id,
        org_id,
        kind,
        subject_id,
        metadata
    )
    SELECT
        uuidv7(),
        'principal_' || REPLACE(uuidv7()::text, '-', ''),
        o.org_id,
        'system',
        ('00000000-0000-0000-0000-' || LPAD(TO_HEX(o.org_id), 12, '0'))::uuid,
        jsonb_build_object('source', 'system_backfill', 'reason', 'missing_human_owner')
    FROM org_fallback_users o
    WHERE o.fallback_user_id IS NULL
    ON CONFLICT (org_id, kind, subject_id) DO NOTHING
    RETURNING 1
)
SELECT COUNT(*) FROM inserted;

WITH org_fallback_users AS (
    SELECT
        o.org_id,
        COALESCE(
            o.created_by,
            (
                SELECT om.user_id
                FROM organization_members om
                WHERE om.org_id = o.org_id AND om.role = 'owner'
                ORDER BY om.created_at ASC
                LIMIT 1
            ),
            (
                SELECT om.user_id
                FROM organization_members om
                WHERE om.org_id = o.org_id
                ORDER BY om.created_at ASC
                LIMIT 1
            )
        ) AS fallback_user_id
    FROM organizations o
),
fallback_user_principals AS (
    SELECT p.org_id, p.id, p.subject_id AS user_id
    FROM principals p
    WHERE p.kind = 'user'
),
fallback_system_principals AS (
    SELECT p.org_id, p.id
    FROM principals p
    WHERE p.kind = 'system'
),
inserted AS (
    INSERT INTO principals (
        id,
        public_id,
        org_id,
        kind,
        subject_id,
        parent_principal_id,
        resolved_user_id,
        metadata
    )
    SELECT
        uuidv7(),
        'principal_' || REPLACE(uuidv7()::text, '-', ''),
        ai.org_id,
        'agent_identity',
        ai.id,
        COALESCE(fup.id, fsp.id),
        ofu.fallback_user_id,
        jsonb_build_object(
            'source', 'agent_identity_backfill',
            'name', ai.name,
            'description', ai.description,
            'avatar_url', ai.avatar_url,
            'backfill_owner_user_id', ofu.fallback_user_id
        )
    FROM agent_identities ai
    JOIN org_fallback_users ofu ON ofu.org_id = ai.org_id
    LEFT JOIN fallback_user_principals fup
        ON fup.org_id = ai.org_id AND fup.user_id = ofu.fallback_user_id
    LEFT JOIN fallback_system_principals fsp
        ON fsp.org_id = ai.org_id
    ON CONFLICT (org_id, kind, subject_id) DO NOTHING
    RETURNING 1
)
SELECT COUNT(*) FROM inserted;

----------------------------------------------------------------------
-- Backfill owned entities
----------------------------------------------------------------------

WITH session_event_owner AS (
    SELECT DISTINCT ON (e.session_id)
        e.session_id,
        COALESCE(
            NULLIF(e.metadata->'acting_principal'->>'user_id', '')::uuid,
            NULLIF(e.metadata->'initiator'->>'user_id', '')::uuid
        ) AS user_id
    FROM events e
    WHERE e.metadata IS NOT NULL
      AND (
        e.metadata->'acting_principal'->>'type' = 'user'
        OR e.metadata->'initiator'->>'type' = 'user'
      )
    ORDER BY e.session_id, e.ts ASC
),
session_tag_owner AS (
    SELECT
        s.id AS session_id,
        NULLIF(SUBSTRING(t.tag FROM '^user:([0-9a-fA-F-]{36})$'), '')::uuid AS user_id
    FROM sessions s
    JOIN LATERAL UNNEST(s.tags) AS t(tag) ON TRUE
    WHERE t.tag ~ '^user:[0-9a-fA-F-]{36}$'
),
org_fallback_users AS (
    SELECT
        o.org_id,
        COALESCE(
            o.created_by,
            (
                SELECT om.user_id
                FROM organization_members om
                WHERE om.org_id = o.org_id AND om.role = 'owner'
                ORDER BY om.created_at ASC
                LIMIT 1
            ),
            (
                SELECT om.user_id
                FROM organization_members om
                WHERE om.org_id = o.org_id
                ORDER BY om.created_at ASC
                LIMIT 1
            )
        ) AS fallback_user_id
    FROM organizations o
),
user_principals AS (
    SELECT org_id, id, subject_id AS user_id
    FROM principals
    WHERE kind = 'user'
),
agent_identity_principals AS (
    SELECT org_id, id, subject_id AS agent_identity_id, resolved_user_id
    FROM principals
    WHERE kind = 'agent_identity'
),
system_principals AS (
    SELECT org_id, id
    FROM principals
    WHERE kind = 'system'
)
UPDATE sessions s
SET
    owner_principal_id = COALESCE(aip.id, up.id, sp.id),
    resolved_owner_user_id = COALESCE(aip.resolved_user_id, chosen_user_id)
FROM (
    SELECT
        s.id,
        s.org_id,
        s.agent_identity_id,
        COALESCE(sto.user_id, seo.user_id, ofu.fallback_user_id) AS chosen_user_id
    FROM sessions s
    LEFT JOIN session_tag_owner sto ON sto.session_id = s.id
    LEFT JOIN session_event_owner seo ON seo.session_id = s.id
    LEFT JOIN org_fallback_users ofu ON ofu.org_id = s.org_id
) derived
LEFT JOIN agent_identity_principals aip
    ON aip.org_id = derived.org_id AND aip.agent_identity_id = derived.agent_identity_id
LEFT JOIN user_principals up
    ON up.org_id = derived.org_id AND up.user_id = derived.chosen_user_id
LEFT JOIN system_principals sp
    ON sp.org_id = derived.org_id
WHERE s.id = derived.id
  AND s.owner_principal_id IS NULL;

WITH org_fallback_users AS (
    SELECT
        o.org_id,
        COALESCE(
            o.created_by,
            (
                SELECT om.user_id
                FROM organization_members om
                WHERE om.org_id = o.org_id AND om.role = 'owner'
                ORDER BY om.created_at ASC
                LIMIT 1
            ),
            (
                SELECT om.user_id
                FROM organization_members om
                WHERE om.org_id = o.org_id
                ORDER BY om.created_at ASC
                LIMIT 1
            )
        ) AS fallback_user_id
    FROM organizations o
),
user_principals AS (
    SELECT org_id, id, subject_id AS user_id
    FROM principals
    WHERE kind = 'user'
),
agent_identity_principals AS (
    SELECT org_id, id, subject_id AS agent_identity_id, resolved_user_id
    FROM principals
    WHERE kind = 'agent_identity'
),
system_principals AS (
    SELECT org_id, id
    FROM principals
    WHERE kind = 'system'
)
UPDATE apps a
SET
    owner_principal_id = COALESCE(aip.id, up.id, sp.id),
    resolved_owner_user_id = COALESCE(aip.resolved_user_id, derived.fallback_user_id)
FROM (
    SELECT
        a.id,
        a.org_id,
        a.agent_identity_id,
        ofu.fallback_user_id
    FROM apps a
    LEFT JOIN org_fallback_users ofu ON ofu.org_id = a.org_id
) derived
LEFT JOIN agent_identity_principals aip
    ON aip.org_id = derived.org_id AND aip.agent_identity_id = derived.agent_identity_id
LEFT JOIN user_principals up
    ON up.org_id = derived.org_id AND up.user_id = derived.fallback_user_id
LEFT JOIN system_principals sp
    ON sp.org_id = derived.org_id
WHERE a.id = derived.id
  AND a.owner_principal_id IS NULL;

UPDATE session_schedules ss
SET
    owner_principal_id = s.owner_principal_id,
    resolved_owner_user_id = s.resolved_owner_user_id
FROM sessions s
WHERE s.id = ss.session_id
  AND ss.owner_principal_id IS NULL;

ALTER TABLE sessions
    ALTER COLUMN owner_principal_id SET NOT NULL;

ALTER TABLE session_schedules
    ALTER COLUMN owner_principal_id SET NOT NULL;

ALTER TABLE apps
    ALTER COLUMN owner_principal_id SET NOT NULL;
