-- Session participant identity: one active agent-member row per agent in a session.
--
-- Participant rows preserve leave history, but active membership should not be
-- polluted by repeated POSTs for the same agent. Keep the earliest active row
-- and close duplicates before adding the partial uniqueness constraint.

WITH ranked_active_agents AS (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY session_id, agent_id
            ORDER BY joined_at ASC, created_at ASC, id ASC
        ) AS duplicate_rank
    FROM session_participants
    WHERE kind = 'agent'
      AND role = 'member'
      AND left_at IS NULL
      AND agent_id IS NOT NULL
)
UPDATE session_participants
SET left_at = NOW(),
    updated_at = NOW()
WHERE id IN (
    SELECT id
    FROM ranked_active_agents
    WHERE duplicate_rank > 1
);

CREATE UNIQUE INDEX session_participants_one_active_agent_member_idx
    ON session_participants(session_id, agent_id)
    WHERE kind = 'agent' AND role = 'member' AND left_at IS NULL;
