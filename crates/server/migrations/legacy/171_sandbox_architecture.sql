-- Reusable Environment configuration, immutable revisions, and durable Sandbox roles.

CREATE TABLE execution_environments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT,
    is_managed BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'archived')),
    current_revision_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX execution_environments_org_name_active_uq
    ON execution_environments (org_id, name)
    WHERE status = 'active';

CREATE TABLE execution_environment_revisions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    environment_id UUID NOT NULL REFERENCES execution_environments(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision > 0),
    profile JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (environment_id, revision)
);

ALTER TABLE execution_environments
    ADD CONSTRAINT execution_environments_current_revision_fk
    FOREIGN KEY (current_revision_id) REFERENCES execution_environment_revisions(id);

ALTER TABLE sandboxes
    -- Start legacy rows as resources so the temporary multi-provider shape
    -- cannot violate the one-primary invariant while it is classified below.
    ADD COLUMN role TEXT NOT NULL DEFAULT 'resource'
        CHECK (role IN ('primary', 'resource')),
    ADD COLUMN environment_revision_id UUID
        REFERENCES execution_environment_revisions(id);

WITH ranked AS (
    SELECT id,
           row_number() OVER (
               PARTITION BY session_id
               ORDER BY (profile_snapshot IS NOT NULL) DESC,
                        (current_instance_id IS NOT NULL) DESC,
                        updated_at DESC,
                        id
           ) AS rank
    FROM sandboxes
)
UPDATE sandboxes sandbox
SET role = 'primary'
FROM ranked
WHERE sandbox.id = ranked.id AND ranked.rank = 1;

CREATE UNIQUE INDEX sandboxes_one_primary_per_session
    ON sandboxes (session_id) WHERE role = 'primary';

-- Every new implicit shell/files Sandbox is primary. Explicit fleet resources
-- are registered through Session resources and always set their role.
ALTER TABLE sandboxes ALTER COLUMN role SET DEFAULT 'primary';

COMMENT ON TABLE execution_environments IS
    'Reusable org-scoped execution configuration. Mutable metadata points at an immutable current revision.';
COMMENT ON TABLE execution_environment_revisions IS
    'Immutable execution profile revisions pinned by Agent versions and Sessions.';
COMMENT ON COLUMN sandboxes.role IS
    'primary is implicitly addressed by shell/files; resource Sandboxes require explicit ids.';

-- Promote legacy Agent-embedded profiles into reusable immutable resources.
-- Keep the complete snapshot on the Agent version during the compatibility
-- window, but attach its revision identity and explicit deterministic policy.
CREATE TEMP TABLE migrated_environment_profiles ON COMMIT DROP AS
SELECT a.id AS agent_id,
       a.org_id,
       profile.key AS profile_name,
       profile.value AS profile,
       gen_random_uuid() AS environment_id,
       gen_random_uuid() AS revision_id,
       row_number() OVER (PARTITION BY a.org_id ORDER BY a.id, profile.key) AS ordinal
FROM agents a
CROSS JOIN LATERAL jsonb_each(COALESCE(a.environments->'profiles', '{}'::jsonb)) profile
WHERE a.environments IS NOT NULL;

INSERT INTO execution_environments
    (id, org_id, name, display_name, description, current_revision_id)
SELECT environment_id,
       org_id,
       'legacy-' || substr(replace(agent_id::text, '-', ''), 1, 12) || '-' ||
           left(profile_name, 24) || '-' || substr(replace(revision_id::text, '-', ''), 1, 8),
       initcap(replace(profile_name, '-', ' ')),
       'Migrated from an Agent environment profile.',
       NULL
FROM migrated_environment_profiles;

INSERT INTO execution_environment_revisions
    (id, environment_id, revision, profile)
SELECT revision_id, environment_id, 1, profile
FROM migrated_environment_profiles;

UPDATE execution_environments environment
SET current_revision_id = migrated.revision_id
FROM migrated_environment_profiles migrated
WHERE environment.id = migrated.environment_id;

UPDATE agents agent
SET environments = jsonb_set(
    jsonb_set(
        agent.environments,
        '{policy}',
        to_jsonb(CASE
            WHEN (
                SELECT count(*)
                FROM jsonb_object_keys(agent.environments->'profiles')
            ) <= 1 THEN 'fixed'::text
            ELSE 'selectable'::text
        END),
        true
    ),
    '{profiles}',
    (
        SELECT jsonb_object_agg(
            migrated.profile_name,
            migrated.profile || jsonb_build_object(
                'source_revision_id',
                'envrev_' || replace(migrated.revision_id::text, '-', '')
            )
        )
        FROM migrated_environment_profiles migrated
        WHERE migrated.agent_id = agent.id
    ),
    true
)
WHERE agent.environments IS NOT NULL
  AND EXISTS (
      SELECT 1 FROM migrated_environment_profiles migrated WHERE migrated.agent_id = agent.id
  );
