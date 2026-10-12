-- Retire Agent Versions (change-history phase 5).
--
-- Agent versions mixed three roles: a change log (automatic draft snapshots),
-- releases (published semver versions, one the default) and runtime binding
-- (endpoints, triggers, participants and sessions pinned to a version). Entity
-- history now owns the change log, and the other two roles are dropped: every
-- exposure runs the agent's current configuration. See
-- knowledge/execution/change-reasons-and-manager-context.md ("Revisions,
-- restore and secrets").
--
-- What this migration does, in order:
--
-- 1. Reports, per org, what was pinned. Pins switch to the current agent;
--    nothing else about the exposure changes.
-- 2. Copies every agent_versions row into the agent's entity history as a
--    revision (actor `system`, surface `internal`, command
--    `migrate_agent_versions`), with the version's summary as the reason, so
--    "go back to how it was" still works through `everruns history restore`.
--    The snapshot has the shape `get_agent` returns: the version's authored
--    configuration over the agent's current values for fields versions never
--    captured (intro, starters, status, ...), so a restore of a migrated
--    revision does not clear them. `mcp_servers` becomes `mcpServers`, the
--    agent's wire name.
-- 3. Renumbers each affected agent's revisions in created_at order, so
--    migrated versions and the history recorded since phases 1-4 form one
--    timeline; a restore entry's `restored_from_revision` follows its revision.
--    The snapshot hash of a migrated revision is not the server's canonical
--    hash: the only use of the hash is "is the next update a no-op", and a
--    mismatch merely records one extra revision.
-- 4. Records on each session the revision it started on (`agent_revision`):
--    a session that captured a version gets that version's revision.
-- 5. Drops the pins, `agents.default_version_id`,
--    `agents.forked_from_version_id`, `sessions.agent_config_hash`,
--    `trace_scores.agent_version_id` and the agent_versions table. Fork
--    lineage (`forked_from_agent_id`, `root_agent_id`) stays.

-- 1. Report pinned exposures per org before switching them to current.
DO $$
DECLARE
    pinned RECORD;
BEGIN
    FOR pinned IN
        SELECT org_id,
               count(*) FILTER (WHERE kind = 'channel') AS channels,
               count(*) FILTER (WHERE kind = 'trigger') AS triggers,
               count(*) FILTER (WHERE kind = 'participant') AS participants,
               count(*) FILTER (WHERE kind = 'default') AS defaults
        FROM (
            SELECT a.org_id, 'channel' AS kind
            FROM agent_channels c JOIN agents a ON a.id = c.agent_id
            WHERE c.agent_version_policy <> 'default' OR c.agent_version_id IS NOT NULL
            UNION ALL
            SELECT t.org_id, 'trigger'
            FROM agent_triggers t
            WHERE COALESCE(t.agent_version_policy, 'default') <> 'default'
               OR t.agent_version_id IS NOT NULL
            UNION ALL
            SELECT p.org_id, 'participant'
            FROM session_participants p
            WHERE p.agent_version_id IS NOT NULL
            UNION ALL
            SELECT a.org_id, 'default'
            FROM agents a
            WHERE a.default_version_id IS NOT NULL
        ) pins
        GROUP BY org_id
        ORDER BY org_id
    LOOP
        RAISE NOTICE
            'retire agent versions: org % had % pinned channels, % pinned triggers, % pinned participants and % default versions; all now run the current agent',
            pinned.org_id, pinned.channels, pinned.triggers, pinned.participants, pinned.defaults;
    END LOOP;
END $$;

-- 2. Copy every version into its agent's history. Revisions are negative
-- placeholders until step 3 numbers the whole timeline.
INSERT INTO entity_changes (
    id, org_id, entity_kind, entity_ref, command, action, reason, changed_fields,
    actor_kind, surface, revision, snapshot, snapshot_hash, created_at
)
SELECT
    v.id,
    a.org_id,
    'agent',
    a.public_id,
    'migrate_agent_versions',
    CASE v.change_kind WHEN 'fork' THEN 'forked' ELSE 'updated' END,
    left(
        CASE
            WHEN v.is_published THEN
                'Agent version ' || v.version
                    || COALESCE(': ' || NULLIF(btrim(v.summary), ''), '')
            ELSE COALESCE(NULLIF(btrim(v.summary), ''), 'Automatic snapshot ' || v.version)
        END,
        1000
    ),
    '{}',
    'system',
    'internal',
    -row_number() OVER (PARTITION BY v.agent_id ORDER BY v.created_at, v.version_number),
    snap.snapshot,
    'migrated:' || md5(snap.snapshot::text),
    v.created_at
FROM agent_versions v
JOIN agents a ON a.id = v.agent_id
CROSS JOIN LATERAL (
    SELECT jsonb_object_agg(field.key, field.value) AS snapshot
    FROM jsonb_each(
        jsonb_build_object(
            'id', a.public_id,
            'service_virtual_user_id',
                'identity_' || replace(a.virtual_user_id::text, '-', ''),
            'intro_markdown', a.intro_markdown,
            'short_description', a.short_description,
            'starters', CASE
                WHEN jsonb_typeof(a.starters) = 'array' AND jsonb_array_length(a.starters) > 0
                THEN a.starters
            END,
            'status', a.status,
            'forked_from_agent_id',
                (SELECT f.public_id FROM agents f WHERE f.id = a.forked_from_agent_id),
            'root_agent_id',
                (SELECT r.public_id FROM agents r WHERE r.id = a.root_agent_id)
        )
        || (v.authored_config - 'mcp_servers' - 'mcpServers' - 'environments' - 'sandbox_policy')
        || jsonb_build_object(
            'mcpServers',
                COALESCE(v.authored_config -> 'mcp_servers', v.authored_config -> 'mcpServers'),
            'sandbox_policy',
                COALESCE(v.authored_config -> 'sandbox_policy', v.authored_config -> 'environments')
        )
    ) AS field
    WHERE field.value <> 'null'::jsonb
) snap;

-- 3. One timeline per affected agent, numbered by created_at. Existing
-- revisions are positive and migrated placeholders negative, so the old
-- numbers never collide inside one entity's map.
CREATE TEMPORARY TABLE agent_revision_renumbering ON COMMIT DROP AS
SELECT
    c.id,
    c.org_id,
    c.entity_ref,
    c.revision AS old_revision,
    row_number() OVER (
        PARTITION BY c.org_id, c.entity_ref
        ORDER BY c.created_at, c.id
    ) AS new_revision
FROM entity_changes c
WHERE c.entity_kind = 'agent'
  AND c.revision IS NOT NULL
  AND EXISTS (
      SELECT 1
      FROM agent_versions v
      JOIN agents a ON a.id = v.agent_id
      WHERE a.org_id = c.org_id AND a.public_id = c.entity_ref
  );

UPDATE entity_changes c
SET revision = m.new_revision
FROM agent_revision_renumbering m
WHERE c.id = m.id;

-- A restore entry names the revision it brought back; follow the renumbering.
UPDATE entity_changes c
SET restored_from_revision = m.new_revision
FROM agent_revision_renumbering m
WHERE c.entity_kind = 'agent'
  AND c.org_id = m.org_id
  AND c.entity_ref = m.entity_ref
  AND c.restored_from_revision = m.old_revision;

-- Snapshots are kept for the newest 500 revisions per entity, as
-- MAX_SNAPSHOTS_PER_ENTITY does for new changes.
UPDATE entity_changes c
SET snapshot = NULL
FROM (
    SELECT org_id, entity_ref, max(revision) AS latest
    FROM entity_changes
    WHERE entity_kind = 'agent' AND revision IS NOT NULL
    GROUP BY org_id, entity_ref
) l
WHERE c.entity_kind = 'agent'
  AND c.org_id = l.org_id AND c.entity_ref = l.entity_ref
  AND c.revision <= l.latest - 500
  AND c.snapshot IS NOT NULL;

-- 4. The revision each session started on.
ALTER TABLE sessions ADD COLUMN agent_revision BIGINT;

UPDATE sessions s
SET agent_revision = c.revision
FROM entity_changes c
WHERE s.agent_version_id IS NOT NULL
  AND c.id = s.agent_version_id;

COMMENT ON COLUMN sessions.agent_revision IS
    'Revision of the agent''s entity history this session started on; `everruns history show` reproduces that configuration.';

-- 5. Drop the pins and the versions themselves.
ALTER TABLE session_participants DROP CONSTRAINT session_participants_agent_shape;
ALTER TABLE session_participants DROP COLUMN agent_version_id;
ALTER TABLE session_participants
    ADD CONSTRAINT session_participants_agent_shape CHECK (
        (kind = 'agent' AND agent_id IS NOT NULL)
        OR (kind = 'user' AND agent_id IS NULL)
    );

ALTER TABLE sessions
    DROP COLUMN agent_version_id,
    DROP COLUMN agent_config_hash;

ALTER TABLE agent_channels
    DROP COLUMN agent_version_policy,
    DROP COLUMN agent_version_id;

ALTER TABLE agent_triggers
    DROP COLUMN agent_version_policy,
    DROP COLUMN agent_version_id;

ALTER TABLE apps
    DROP COLUMN agent_version_policy,
    DROP COLUMN agent_version_id;

ALTER TABLE trace_scores DROP COLUMN agent_version_id;

ALTER TABLE agents
    DROP COLUMN default_version_id,
    DROP COLUMN forked_from_version_id;

DROP TABLE agent_versions;
