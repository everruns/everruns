-- Session participant identity: one active user row per principal in a session.
--
-- Slack and other channel speakers become explicit user participants keyed by
-- their durable external-actor principal. Historical leave/rejoin intervals stay
-- possible because the uniqueness only applies while `left_at` is NULL.

WITH ranked_active_users AS (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY session_id, principal_id
            ORDER BY joined_at ASC, created_at ASC, id ASC
        ) AS duplicate_rank
    FROM session_participants
    WHERE kind = 'user' AND left_at IS NULL
)
UPDATE session_participants
SET left_at = NOW(),
    updated_at = NOW()
WHERE id IN (
    SELECT id
    FROM ranked_active_users
    WHERE duplicate_rank > 1
);

CREATE UNIQUE INDEX session_participants_one_active_user_principal_idx
    ON session_participants(session_id, principal_id)
    WHERE kind = 'user' AND left_at IS NULL;
