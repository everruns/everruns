-- Two ways for an agent to start other agents (knowledge/runtime-resources/threads.md):
-- `spawn_agents` (one-off calls) and `threads` (followed work), never both on one
-- agent; `coordinator` is a prompt that needs `threads`; `sibling_communication`
-- is opt-in. The retired ids are rewritten here the same way
-- `canonicalize_delegation_capabilities` (crates/contracts/src/capability/delegation.rs)
-- rewrites them on read:
--
--   subagents + agent_handoff        -> spawn_agents {self, agents, max_depth, max_open, max_total}
--   coordination (coordinator role)  -> threads {self, any_agent, agents, max_open, max_total} + coordinator
--                                       (hand-off members move to threads.agents; no spawn_agents)
--   coordination (worker role)       -> dropped (every thread gets the parent tools from the host)
--   run_conversation                 -> sibling_communication
--
-- Data rule: nothing is lost. Every row rewritten or folded is copied first to
-- retired_delegation_capabilities.

CREATE TABLE IF NOT EXISTS retired_delegation_capabilities (
    source text NOT NULL,
    owner_id uuid NOT NULL,
    capability_id character varying(71) NOT NULL,
    "position" integer NOT NULL,
    config jsonb NOT NULL,
    created_at timestamp with time zone NOT NULL,
    retired_at timestamp with time zone DEFAULT now() NOT NULL
);

COMMENT ON TABLE retired_delegation_capabilities IS
    'Capability rows rewritten by the threads_capability migration (subagents, agent_handoff, coordination, run_conversation), kept as they were.';

INSERT INTO retired_delegation_capabilities (source, owner_id, capability_id, "position", config, created_at)
SELECT 'agent', agent_id, capability_id, "position", config, created_at
FROM agent_capabilities
WHERE capability_id IN ('subagents', 'agent_handoff', 'coordination', 'run_conversation');

INSERT INTO retired_delegation_capabilities (source, owner_id, capability_id, "position", config, created_at)
SELECT 'harness', harness_id, capability_id, "position", config, created_at
FROM harness_capabilities
WHERE capability_id IN ('subagents', 'agent_handoff', 'coordination', 'run_conversation');

-- Hand-off members (`targets`, or `members`) with an agent id.
CREATE FUNCTION pg_temp.delegation_members(handoff jsonb) RETURNS jsonb
LANGUAGE sql IMMUTABLE AS $$
    SELECT COALESCE(jsonb_agg(member), '[]'::jsonb)
    FROM jsonb_array_elements(
        COALESCE(handoff -> 'targets', handoff -> 'members', '[]'::jsonb)
    ) AS member
    WHERE jsonb_typeof(member) = 'object' AND member ? 'agent_id'
$$;

-- The new rows for one owner's retired rows.
CREATE FUNCTION pg_temp.delegation_rows(sub jsonb, handoff jsonb, coord jsonb)
RETURNS TABLE (capability_id text, config jsonb)
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    members jsonb := pg_temp.delegation_members(handoff);
    workers jsonb;
    agents jsonb;
    cfg jsonb;
BEGIN
    IF coord IS NOT NULL AND COALESCE(coord ->> 'role', 'coordinator') <> 'worker' THEN
        workers := COALESCE(coord -> 'workers', '[{"id":"self"}]'::jsonb);
        SELECT members || COALESCE(jsonb_agg(jsonb_build_object('agent_id', w ->> 'id')), '[]'::jsonb)
        INTO agents
        FROM jsonb_array_elements(workers) AS w
        WHERE w ->> 'id' NOT IN ('self', 'any')
          AND NOT EXISTS (
              SELECT 1 FROM jsonb_array_elements(members) AS m
              WHERE m ->> 'agent_id' = w ->> 'id'
          );
        cfg := jsonb_build_object(
            'self', EXISTS (SELECT 1 FROM jsonb_array_elements(workers) AS w WHERE w ->> 'id' = 'self')
        );
        IF EXISTS (SELECT 1 FROM jsonb_array_elements(workers) AS w WHERE w ->> 'id' = 'any') THEN
            cfg := cfg || '{"any_agent": true}'::jsonb;
        END IF;
        IF jsonb_array_length(agents) > 0 THEN
            cfg := cfg || jsonb_build_object('agents', agents);
        END IF;
        IF coord ? 'max_active_threads' THEN
            cfg := cfg || jsonb_build_object('max_open', coord -> 'max_active_threads');
        END IF;
        IF coord ? 'max_total_threads' THEN
            cfg := cfg || jsonb_build_object('max_total', coord -> 'max_total_threads');
        END IF;
        capability_id := 'threads'; config := cfg; RETURN NEXT;
        capability_id := 'coordinator'; config := '{}'::jsonb; RETURN NEXT;
        RETURN;
    END IF;
    -- A hand-off with no members and no subagents names nobody to spawn.
    IF sub IS NOT NULL OR jsonb_array_length(members) > 0 THEN
        cfg := jsonb_build_object('self', sub IS NOT NULL);
        IF jsonb_array_length(members) > 0 THEN
            cfg := cfg || jsonb_build_object('agents', members);
        END IF;
        IF sub IS NOT NULL THEN
            IF sub ? 'max_subagent_depth' OR sub ? 'max_depth' THEN
                cfg := cfg || jsonb_build_object('max_depth', COALESCE(sub -> 'max_subagent_depth', sub -> 'max_depth'));
            END IF;
            IF sub ? 'max_active_descendant_tasks' OR sub ? 'max_concurrent_descendant_tasks' THEN
                cfg := cfg || jsonb_build_object('max_open', COALESCE(sub -> 'max_active_descendant_tasks', sub -> 'max_concurrent_descendant_tasks'));
            END IF;
            IF sub ? 'max_total_descendant_tasks' THEN
                cfg := cfg || jsonb_build_object('max_total', sub -> 'max_total_descendant_tasks');
            END IF;
            IF sub ? 'helper_models' THEN
                cfg := cfg || jsonb_build_object('helper_models', sub -> 'helper_models');
            END IF;
            IF sub ? 'helper_model_pick' THEN
                cfg := cfg || jsonb_build_object('helper_model_pick', sub -> 'helper_model_pick');
            END IF;
        END IF;
        capability_id := 'spawn_agents'; config := cfg; RETURN NEXT;
    END IF;
END
$$;

-- Agents.
WITH owners AS (
    SELECT
        agent_id AS owner_id,
        min("position") FILTER (WHERE capability_id IN ('subagents', 'agent_handoff', 'coordination')) AS pos,
        (array_agg(config) FILTER (WHERE capability_id = 'subagents'))[1] AS sub,
        (array_agg(config) FILTER (WHERE capability_id = 'agent_handoff'))[1] AS handoff,
        (array_agg(config) FILTER (WHERE capability_id = 'coordination'))[1] AS coord
    FROM agent_capabilities
    WHERE capability_id IN ('subagents', 'agent_handoff', 'coordination')
    GROUP BY agent_id
),
fresh AS (
    SELECT o.owner_id, o.pos, r.capability_id, r.config
    FROM owners o, LATERAL pg_temp.delegation_rows(o.sub, o.handoff, o.coord) r
)
INSERT INTO agent_capabilities (agent_id, capability_id, "position", config)
SELECT owner_id, capability_id, pos, config FROM fresh
ON CONFLICT (agent_id, capability_id) DO NOTHING;

-- A coordinator keeps its hand-off agents on Threads: never Spawn agents too.
DELETE FROM agent_capabilities sa
WHERE sa.capability_id = 'spawn_agents'
  AND EXISTS (
      SELECT 1 FROM agent_capabilities t
      WHERE t.agent_id = sa.agent_id AND t.capability_id = 'threads'
  )
  AND EXISTS (
      SELECT 1 FROM retired_delegation_capabilities r
      WHERE r.source = 'agent' AND r.owner_id = sa.agent_id AND r.capability_id = 'coordination'
  );

UPDATE agent_capabilities a
SET capability_id = 'sibling_communication'
WHERE a.capability_id = 'run_conversation'
  AND NOT EXISTS (
      SELECT 1 FROM agent_capabilities s
      WHERE s.agent_id = a.agent_id AND s.capability_id = 'sibling_communication'
  );

DELETE FROM agent_capabilities
WHERE capability_id IN ('subagents', 'agent_handoff', 'coordination', 'run_conversation');

-- Harnesses (custom ones; built-ins are re-provisioned from code).
WITH owners AS (
    SELECT
        harness_id AS owner_id,
        min("position") FILTER (WHERE capability_id IN ('subagents', 'agent_handoff', 'coordination')) AS pos,
        (array_agg(config) FILTER (WHERE capability_id = 'subagents'))[1] AS sub,
        (array_agg(config) FILTER (WHERE capability_id = 'agent_handoff'))[1] AS handoff,
        (array_agg(config) FILTER (WHERE capability_id = 'coordination'))[1] AS coord
    FROM harness_capabilities
    WHERE capability_id IN ('subagents', 'agent_handoff', 'coordination')
    GROUP BY harness_id
),
fresh AS (
    SELECT o.owner_id, o.pos, r.capability_id, r.config
    FROM owners o, LATERAL pg_temp.delegation_rows(o.sub, o.handoff, o.coord) r
)
INSERT INTO harness_capabilities (harness_id, capability_id, "position", config)
SELECT owner_id, capability_id, pos, config FROM fresh
ON CONFLICT (harness_id, capability_id) DO NOTHING;

UPDATE harness_capabilities h
SET capability_id = 'sibling_communication'
WHERE h.capability_id = 'run_conversation'
  AND NOT EXISTS (
      SELECT 1 FROM harness_capabilities s
      WHERE s.harness_id = h.harness_id AND s.capability_id = 'sibling_communication'
  );

DELETE FROM harness_capabilities
WHERE capability_id IN ('subagents', 'agent_handoff', 'coordination', 'run_conversation');

-- Agents on a harness that inherited `subagents` from the Worker level keep
-- spawning themselves when they actually did: the Worker level no longer
-- carries Spawn agents, so those agents get it on the agent.
INSERT INTO agent_capabilities (agent_id, capability_id, "position", config)
SELECT DISTINCT s.agent_id, 'spawn_agents', 1000, '{"self": true}'::jsonb
FROM session_tasks t
JOIN sessions s ON s.id = t.session_id
WHERE t.kind = 'subagent'
  AND s.agent_id IS NOT NULL
  AND EXISTS (SELECT 1 FROM agents a WHERE a.id = s.agent_id)
  AND NOT EXISTS (
      SELECT 1 FROM agent_capabilities c
      WHERE c.agent_id = s.agent_id AND c.capability_id IN ('spawn_agents', 'threads')
  )
ON CONFLICT (agent_id, capability_id) DO NOTHING;
